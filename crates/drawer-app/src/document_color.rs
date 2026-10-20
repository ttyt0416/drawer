use super::*;
use drawer_core::{
    color_precision::Depth,
    color_profile::RgbProfile,
    color_storage::{self as core, Config, ProfileChange},
};
#[derive(Default)]
pub(super) struct Panel {
    draft: Option<Box<Draft>>,
    preview: Option<egui::Rect>,
    apply: Option<egui::Rect>,
}
struct Draft {
    source: (u64, u64, u32),
    config: Config,
    convert: bool,
    rasterize_sources: bool,
    state: Option<drawer_core::sizing::DocumentState>,
    images: Option<[egui::TextureHandle; 2]>,
    loss: Option<String>,
}
impl Drawer {
    pub(super) fn begin_document_color(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        let mut config = d.color.clone();
        config.calculation = core::Calculation::Float64;
        self.document_color.draft = Some(Box::new(Draft {
            source: (d.id, d.revision, d.current_frame),
            config,
            convert: true,
            rasterize_sources: false,
            state: None,
            images: None,
            loss: None,
        }));
        Ok(())
    }
    fn preview_document_color(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let e = self
            .document_color
            .draft
            .as_ref()
            .ok_or("Color editor closed")?;
        let d = &self.documents[self.active.ok_or("Open a page first")?];
        if e.source != (d.id, d.revision, d.current_frame) {
            return Err("Page/frame changed; reopen color editor".into());
        }
        let used: usize = self.documents.iter().map(Document::resident_bytes).sum();
        if used.saturating_add(d.memory_bytes().saturating_mul(6)) > DEFAULT_DOCUMENT_BUDGET {
            return Err("Color preview exceeds workspace memory".into());
        }
        let state = core::prepare_with_sources(
            d,
            e.config.clone(),
            if e.convert {
                ProfileChange::ConvertPixels
            } else {
                ProfileChange::AssignNumbers
            },
            e.rasterize_sources,
        )?;
        let (mut changed_channels, mut maximum_delta) = (0u64, 0u16);
        for layer in &d.layers {
            let extent = layer.extent(d.width, d.height);
            for n in 0..layer.pixels.len() {
                let sample = layer.sample_at(n);
                let stored = sample.store(
                    e.config.depth,
                    e.config.dither,
                    [
                        extent.offset[0] + (n % extent.width as usize) as i64,
                        extent.offset[1] + (n / extent.width as usize) as i64,
                    ],
                );
                let mapped = if e.config.depth == Depth::Eight {
                    stored.map(|v| v * 257)
                } else {
                    stored
                };
                for (a, b) in sample.rgba16().into_iter().zip(mapped) {
                    let delta = a.abs_diff(b);
                    changed_channels += u64::from(delta != 0);
                    maximum_delta = maximum_delta.max(delta);
                }
            }
        }
        let loss = format!(
            "Depth/dither alone changes {changed_channels} stored raster channels; maximum difference {maximum_delta} of 65535. Profile conversion and rasterization are shown in the page preview."
        );
        let after = drawer_core::layer_merge::preview(&state, d.current_frame)?;
        let images = [
            super::layer_merge::image(ctx, d, "color-before"),
            super::layer_merge::image(ctx, &after, "color-after"),
        ];
        let e = self.document_color.draft.as_mut().unwrap();
        e.state = Some(state);
        e.images = Some(images);
        e.loss = Some(loss);
        Ok(())
    }
    fn apply_document_color(&mut self) -> Result<(), String> {
        let d = &mut self.documents[self.active.ok_or("Open a page first")?];
        let e = self
            .document_color
            .draft
            .as_mut()
            .ok_or("Color editor closed")?;
        if e.source != (d.id, d.revision, d.current_frame) {
            return Err("Page/frame changed; reopen color editor".into());
        }
        d.commit_state(e.state.take().ok_or("Preview the color conversion first")?)?;
        self.document_color.draft = None;
        Ok(())
    }
    pub(super) fn document_color_window(&mut self, ctx: &egui::Context) {
        let Some(mut e) = self.document_color.draft.take() else {
            return;
        };
        let fresh = self.active.is_some_and(|n| {
            let d = &self.documents[n];
            e.source == (d.id, d.revision, d.current_frame)
        });
        let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (mut preview, mut apply, mut cancel, mut load) = (false, false, false, false);
        egui::Window::new("Document precision and RGB profile").open(&mut open).default_pos(egui::pos2(30.,25.)).default_width(515.).collapsible(false).show(ctx,|ui|{
            let mut changed=false;
            ui.allocate_ui(egui::vec2(500.,330.),|ui|{egui::ScrollArea::vertical().max_height(330.).show(ui,|ui|{
            ui.horizontal(|ui|{ui.label("RGBA channel precision");for(depth,label)in[(Depth::Sixteen,"16 bit"),(Depth::Eight,"8 bit")]{changed|=ui.selectable_value(&mut e.config.depth,depth,label).changed();}});
            ui.small("16 to 8 bit loses channel detail. Preview first; Apply stores pixels and profile together in one Undo. PNG export leaves document precision unchanged.");
            changed|=ui.checkbox(&mut e.config.dither.enabled,"Dither final stored RGB").changed();
            ui.add_enabled_ui(e.config.dither.enabled,|ui|{ui.horizontal(|ui|{ui.label("Fixed seed");changed|=ui.add(egui::DragValue::new(&mut e.config.dither.seed)).changed();});});
            ui.label(format!("RGB profile: {}",e.config.profile.label()));
            ui.horizontal(|ui|{if ui.button("sRGB").clicked(){e.config.profile=RgbProfile::Srgb;changed=true;}load=ui.button("Local RGB ICC file…").clicked();});
            changed|=ui.radio_value(&mut e.convert,true,"Convert pixel colors to the chosen RGB profile").changed();
            changed|=ui.radio_value(&mut e.convert,false,"Assign profile; keep pixel numbers").changed();
            changed|=ui.add_enabled(e.convert,egui::Checkbox::new(&mut e.rasterize_sources,"Rasterize all editable sources at the current frame for profile conversion")).changed();
            if e.rasterize_sources&&e.convert{ui.colored_label(egui::Color32::YELLOW,"This replaces the visible page with one raster. Hidden/off-canvas content, vectors, folders and animation are removed; one Undo restores the complete original.");}
            if !e.convert{ui.colored_label(egui::Color32::YELLOW,"Assigning an ICC profile changes what the same numbers mean; appearance can change. Precision conversion still quantizes to the chosen bit depth.");}
            ui.small("Unprofiled RGB is interpreted as sRGB. CMYK is handled through explicit import/export conversion, never as an RGBA document profile. View-only soft proof is a separate command.");
            });});
            if changed{e.state=None;e.images=None;e.loss=None;}
            let r=ui.add_enabled(fresh,egui::Button::new("Preview color conversion"));self.document_color.preview=Some(r.rect);preview=r.clicked();
            ui.allocate_ui(egui::vec2(450.,40.),|ui|{if let Some(loss)=&e.loss{ui.small(loss);}});
            ui.allocate_ui(egui::vec2(300.,112.),|ui|{if let Some(images)=&e.images{ui.horizontal(|ui|{for(label,image)in[("Current page",&images[0]),("Converted page",&images[1])]{ui.vertical(|ui|{ui.label(label);ui.add(egui::Image::new(image).fit_to_exact_size(egui::vec2(96.,96.)));});}});}});
            ui.horizontal(|ui|{let r=ui.add_enabled(fresh&&e.state.is_some(),egui::Button::new("Apply color conversion (one Undo)"));self.document_color.apply=Some(r.rect);apply=r.clicked();cancel=ui.button("Cancel").clicked();});
            if !fresh{ui.label("Page/frame changed; close and reopen this editor.");}
        });
        if open && !cancel {
            self.document_color.draft = Some(e);
        }
        let result = if load {
            self.load_document_icc()
        } else if preview {
            self.preview_document_color(ctx)
        } else if apply {
            self.apply_document_color()
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.status = Some(error);
        }
    }
    fn load_document_icc(&mut self) -> Result<(), String> {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("RGB ICC profile", &["icc", "icm"])
            .pick_file()
        else {
            return Ok(());
        };
        if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
            return Err("RGB ICC exceeds 4MiB".into());
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("RGB ICC grew beyond 4MiB while reading".into());
        }
        let name = path
            .file_name()
            .ok_or("ICC filename missing")?
            .to_string_lossy()
            .into_owned();
        let profile = RgbProfile::embedded(name, bytes)?;
        let e = self
            .document_color
            .draft
            .as_mut()
            .ok_or("Color editor closed")?;
        e.config.profile = profile;
        e.state = None;
        e.images = None;
        e.loss = None;
        Ok(())
    }
}

pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    use drawer_core::color_precision::Sample;
    drawer_core::color_precision_verify::verify()?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.create_canvas_color(
        CanvasSpec {
            width: 2,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        Config::default(),
    )
    .map_err(|e| e.to_string())?;
    let pixels = vec![[12345, 23456, 34567, 1], [54321, 43210, 32109, 32768]];
    app.documents[0].layers[0].pixels = pixels.iter().map(|p| Sample::from16(*p).rgba8()).collect();
    app.documents[0].layers[0].precise = Some(pixels);
    let original = app.documents[0].snapshot();
    let file = root.join("precision.drawer");
    drawer_core::files::save(&mut app.documents[0], &file).map_err(|e| e.to_string())?;
    if drawer_core::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != original
    {
        return Err("Restart lost precise channels".into());
    }
    app.execute_command(&ctx, "document-color");
    app.document_color
        .draft
        .as_mut()
        .ok_or("Color command missing")?
        .config
        .depth = Depth::Eight;
    let mut t = 0.;
    for _ in 0..3 {
        t += 0.1;
        super::input_replay::frame(&ctx, &mut app, t, vec![]);
    }
    let preview = app
        .document_color
        .preview
        .ok_or("Color preview button missing")?;
    let click = |app: &mut Drawer, t: &mut f64, r: egui::Rect| {
        for pressed in [true, false] {
            *t += 0.1;
            super::input_replay::frame(
                &ctx,
                app,
                *t,
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
    click(&mut app, &mut t, preview);
    if app.documents[0].snapshot() != original
        || app
            .document_color
            .draft
            .as_ref()
            .is_none_or(|d| d.state.is_none())
    {
        return Err(format!(
            "Color preview changed source or failed: {:?}",
            app.status
        ));
    }
    for _ in 0..2 {
        t += 0.1;
        super::input_replay::frame(&ctx, &mut app, t, vec![]);
    }
    let apply = app
        .document_color
        .apply
        .ok_or("Color apply button missing")?;
    click(&mut app, &mut t, apply);
    if app.document_color.draft.is_some()
        || app.documents[0].color.depth != Depth::Eight
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != original
    {
        return Err(format!("Precision Apply/Undo failed: {:?}", app.status));
    }
    drawer_core::png_export::export(
        &app.documents[0],
        &root.join("precision.png"),
        DEFAULT_DOCUMENT_BUDGET / 4,
    )
    .map_err(|e| e.to_string())?;
    if app.documents[0].snapshot() != original {
        return Err("PNG export changed document precision".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mouse_and_virtual_pen_create_one_precise_stroke_and_undo_original() {
        use drawer_core::pointer_input::{Kind, Phase, Source};
        for pen in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Drawer::new(&ctx, None);
            app.simple_view = true;
            app.create_canvas_color(
                CanvasSpec {
                    width: 8,
                    height: 8,
                    dpi: 300.,
                    background: Background::Transparent,
                },
                Config::default(),
            )
            .unwrap();
            app.set_tool(BrushTip::Marker);
            app.panels.settings.brush_size = 2.;
            app.panels.settings.brush_opacity = [0.01; 7];
            app.panels.settings.brush_hardness = [1.; 7];
            app.workspaces.store.current.input.smoothing = [0.; 7];
            app.references.session.workspace.color = [31, 67, 109, 255];
            for i in 0..3 {
                crate::input_replay::frame(&ctx, &mut app, i as f64 * 0.1, vec![]);
            }
            let original = app.documents[0].snapshot();
            for (i, phase) in [Phase::Down, Phase::Up].into_iter().enumerate() {
                let mut event = crate::precision::pointer(&ctx, &app, [4.5; 2], phase);
                if pen {
                    event.source = Source {
                        kind: Kind::Pen,
                        device: 71,
                        contact: 9,
                    };
                    event.pressure = Some(0.5);
                }
                app.drawing.virtual_events(vec![event], pen);
                crate::input_replay::frame(&ctx, &mut app, 0.5 + i as f64 * 0.1, vec![]);
            }
            assert_eq!(
                app.documents[0].history_counts(),
                (1, 0),
                "{:?}",
                app.status
            );
            assert!(
                app.documents[0].layers[0]
                    .precise
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|p| p[3] > 0 && p[3] % 257 != 0)
            );
            assert!(app.documents[0].undo());
            assert_eq!(app.documents[0].snapshot(), original);
        }
    }
    #[test]
    fn new_canvas_ui_creates_sixteen_by_default_and_float_eight_when_selected() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        for (depth, n) in [(Depth::Sixteen, 0), (Depth::Eight, 1)] {
            if n == 0 {
                assert_eq!(app.new_canvas.depth, depth);
            } else {
                app.new_canvas.depth = depth;
            }
            app.new_canvas.size = drawer_core::canvas_size::Dimensions::new(2, 1, 300.);
            app.show_new_canvas = true;
            let time = n as f64 * 2.;
            for i in 0..3 {
                crate::input_replay::frame(&ctx, &mut app, time + i as f64 * 0.1, vec![]);
            }
            let r = app.create_button.unwrap();
            for (i, pressed) in [true, false].into_iter().enumerate() {
                crate::input_replay::frame(
                    &ctx,
                    &mut app,
                    time + 0.5 + i as f64 * 0.1,
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
            assert!(!app.show_new_canvas, "{:?}", app.new_canvas.error);
            assert_eq!(app.documents[n].color.depth, depth);
            assert_eq!(
                app.documents[n].color.calculation,
                core::Calculation::Float64
            );
            assert_eq!(
                app.documents[n].layers[0].precise.is_some(),
                depth == Depth::Sixteen
            );
        }
    }
    #[test]
    fn precision_dialog_preview_apply_undo_stale_and_escape_and_new_canvas_default() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.simple_view = true;
        assert_eq!(app.new_canvas.depth, Depth::Sixteen);
        app.create_canvas_color(
            CanvasSpec {
                width: 2,
                height: 1,
                dpi: 300.,
                background: Background::Transparent,
            },
            Config::default(),
        )
        .unwrap();
        let p = [12345, 23456, 34567, 123];
        app.documents[0].layers[0].precise.as_mut().unwrap()[0] = p;
        app.documents[0].layers[0].pixels[0] =
            drawer_core::color_precision::Sample::from16(p).rgba8();
        let original = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let mut t = 0.;
        let frame = |app: &mut Drawer, t: &mut f64, events| {
            *t += 0.1;
            crate::input_replay::frame(&ctx, app, *t, events);
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
        };
        app.execute_command(&ctx, "document-color");
        app.document_color.draft.as_mut().unwrap().config.depth = Depth::Eight;
        for _ in 0..3 {
            frame(&mut app, &mut t, vec![]);
        }
        let r = app.document_color.preview.unwrap();
        click(&mut app, &mut t, r);
        assert!(
            app.document_color.draft.as_ref().unwrap().state.is_some(),
            "{:?}",
            app.status
        );
        assert_eq!(app.documents[0].snapshot(), original);
        assert_eq!(app.documents[0].history_counts(), counts);
        for _ in 0..2 {
            frame(&mut app, &mut t, vec![]);
        }
        let r = app.document_color.apply.unwrap();
        click(&mut app, &mut t, r);
        assert!(app.document_color.draft.is_none(), "{:?}", app.status);
        assert_eq!(app.documents[0].color.depth, Depth::Eight);
        assert!(app.documents[0].layers[0].precise.is_none());
        assert!(app.documents[0].undo());
        assert_eq!(app.documents[0].snapshot(), original);
        app.begin_document_color().unwrap();
        app.preview_document_color(&ctx).unwrap();
        app.documents[0].edit_pixel(0, 1, 0, [2, 3, 4, 255]);
        let changed = app.documents[0].snapshot();
        assert!(app.apply_document_color().is_err());
        assert_eq!(app.documents[0].snapshot(), changed);
        frame(
            &mut app,
            &mut t,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(app.document_color.draft.is_none());
    }
}
