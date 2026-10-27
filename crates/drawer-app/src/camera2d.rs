use super::*;
use drawer_core::{
    camera2d::{self as core, Guides},
    keyframes::{self, Owner},
    layer_search::Key,
};
#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    notice: String,
    toggle: Option<egui::Rect>,
    guide: Option<egui::Rect>,
    keys: Option<egui::Rect>,
    base: Option<egui::Rect>,
}
#[derive(Default)]
pub(super) struct Paths {
    stamp: Option<(u64, u64, u32, bool, Guides)>,
    frames: Vec<core::Frame>,
}
impl Paths {
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        d: &drawer_core::document::Document,
        view: drawer_core::view::View,
        rect: egui::Rect,
        ppp: f64,
    ) -> Result<(), String> {
        let g = d.camera_guides;
        let stamp = (
            d.id,
            d.revision,
            d.current_frame,
            keyframes::content_view(d),
            g,
        );
        if self.stamp != Some(stamp) {
            self.frames = core::paths(d)?;
            self.stamp = Some(stamp);
        }
        let size = [d.width as f64, d.height as f64];
        let center = [rect.center().x as f64, rect.center().y as f64];
        let screen = |p| {
            let p = view.document_to_screen(p, size, center, ppp);
            egui::pos2(p[0] as f32, p[1] as f32)
        };
        let mut previous: [Option<&core::Frame>; 2] = [None, None];
        for f in &self.frames {
            let stroke = egui::Stroke::new(
                1.,
                egui::Color32::from_rgb(f.color[0], f.color[1], f.color[2]),
            );
            let c = f.corners.map(screen);
            painter.add(egui::Shape::closed_line(c.to_vec(), stroke));
            if g.centers {
                painter.circle_stroke(screen(f.center), 2.5, stroke);
            }
            let i = usize::from(f.camera);
            if let Some(old) = previous[i] {
                if g.corners {
                    for (a, b) in old.corners.into_iter().zip(c) {
                        painter.line_segment([screen(a), b], stroke);
                    }
                }
                if g.centers {
                    painter.line_segment([screen(old.center), screen(f.center)], stroke);
                }
            }
            previous[i] = Some(f);
        }
        Ok(())
    }
}
impl Drawer {
    pub(super) fn begin_camera2d(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &self.documents[self.active.ok_or("Choose a document")?];
        d.animation.as_ref().ok_or("Create a timeline first")?;
        self.camera2d.open = true;
        self.camera2d.notice.clear();
        self.stop_timeline_playback(d.id);
        Ok(())
    }
    pub(super) fn camera_input_ready(&self) -> bool {
        self.active.is_none_or(|n| {
            let d = &self.documents[n];
            !d.scene.keyframes.camera_output || keyframes::content_view(d)
        })
    }
    fn camera_base(&mut self) -> Result<(), String> {
        self.finish_stroke();
        let d = &mut self.documents[self.active.ok_or("Choose a document")?];
        let owner = if keyframes::content_view(d) {
            None
        } else {
            Some(Owner::Layer(Key::Layer(
                drawer_core::layer_properties::raster(d, 0),
            )))
        };
        keyframes::edit_content(d, owner)
    }
    pub(super) fn camera2d_window(&mut self, ctx: &egui::Context) {
        if !self.camera2d.open {
            return;
        }
        let Some(n) = self.active else { return };
        let d = &self.documents[n];
        let Some(a) = &d.animation else { return };
        let mut p = std::mem::take(&mut self.camera2d);
        let mut open = p.open;
        let mut on = d.scene.keyframes.camera_output;
        let mut g = d.camera_guides;
        let mut keys = false;
        let mut base = false;
        egui::Window::new("2D camera and path guides").id(egui::Id::new("camera-2d")).open(&mut open).default_pos(egui::pos2(75.,60.)).default_width(640.).resizable(true).vscroll(true).show(ctx,|ui|{
            ui.label(&p.notice);
            let r=ui.checkbox(&mut on,"Apply camera to final composition / exports");p.toggle=Some(r.rect);
            ui.label("Camera X/Y moves the filming frame through the world. Scale1 is original size; larger scales show a wider area (zoom =1/scale). Positive rotation turns the camera frame clockwise.");
            ui.horizontal(|ui|{let r=ui.button("Edit camera keys and interpolation curves");p.keys=Some(r.rect);keys=r.clicked();let r=ui.button(if keyframes::content_view(d){"Return to camera view"}else{"Edit source content"});p.base=Some(r.rect);base=r.clicked();});
            ui.label("Draw in source content view. Existing parent-camera links keep layers attached to the camera; unlinked layers are filmed by it. Guides are never exported.");
            ui.separator();let r=ui.checkbox(&mut g.enabled,"Show path / frame guides");p.guide=Some(r.rect);
            ui.horizontal(|ui|{ui.checkbox(&mut g.camera,"Camera path");ui.checkbox(&mut g.keys_only,"Keyframes only");ui.checkbox(&mut g.corners,"Connect frame corners");ui.checkbox(&mut g.centers,"Connect rotation centers");});
            let rows=drawer_core::layer_catalog::rows(d).unwrap_or_default();
            egui::ComboBox::from_id_salt("camera-guide-layer").selected_text(g.layer.and_then(|k|rows.iter().find(|r|r.key==k).map(|r|r.name.as_str())).unwrap_or("No layer path")).show_ui(ui,|ui|{ui.selectable_value(&mut g.layer,None,"No layer path");for row in &rows{ui.selectable_value(&mut g.layer,Some(row.key),&row.name);}});
            ui.horizontal(|ui|{let mut first=g.first+1;let mut last=g.last+1;ui.label("Frame range (included)");ui.add(egui::DragValue::new(&mut first).range(1..=10000));ui.add(egui::DragValue::new(&mut last).range(1..=10000));g.first=first-1;g.last=last-1;});
            ui.horizontal(|ui|{ui.label("Start / end colors (by frame time)");ui.color_edit_button_srgb(&mut g.colors[0]);ui.color_edit_button_srgb(&mut g.colors[1]);});
            ui.label(format!("Timeline: {} frames. Guides outside it are hidden. All-frame guides are bounded to10000 frames per path.",a.frames));
        });
        p.open = open;
        let changed = on != d.scene.keyframes.camera_output;
        let guides = g != d.camera_guides;
        let result = (|| {
            if changed {
                self.finish_stroke();
                let d = &mut self.documents[n];
                let s = core::enable(d, on)?;
                d.commit_state(s)?;
            }
            if guides {
                core::set_guides(&mut self.documents[n], g)?;
            }
            if base {
                self.camera_base()?;
            }
            if keys {
                self.begin_camera_keyframes()?;
                p.open = false;
            }
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            p.notice = e;
        }
        self.camera2d = p;
    }
}
pub(super) fn smoke(root: &std::path::Path) -> Result<(), String> {
    drawer_core::camera2d_verify::verify(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.add_document(drawer_core::onion_verify::fixture());
    app.execute_command(&ctx, "camera-2d");
    if !app.camera2d.open {
        return Err("Camera command missing".into());
    }
    let mut time = 0.;
    let frame = |app: &mut Drawer, time: &mut f64, events| {
        *time += 0.05;
        input_replay::frame(&ctx, app, *time, events);
    };
    let click = |app: &mut Drawer, time: &mut f64, r: egui::Rect| {
        for pressed in [true, false] {
            frame(
                app,
                time,
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
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app.camera2d.toggle.ok_or("Camera enable control missing")?;
    click(&mut app, &mut time, r);
    if !app.documents[0].scene.keyframes.camera_output || app.camera_input_ready() {
        return Err(format!(
            "Camera enable / drawing guard failed: {}",
            app.camera2d.notice
        ));
    }
    let snap = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    let revision = app.documents[0].revision;
    let r = app.camera2d.guide.unwrap();
    click(&mut app, &mut time, r);
    if !app.documents[0].camera_guides.enabled
        || app.documents[0].snapshot() != snap
        || app.documents[0].history_counts() != history
        || app.documents[0].revision != revision
    {
        return Err("Guide view leaked into drawing history".into());
    }
    let r = app.camera2d.base.unwrap();
    click(&mut app, &mut time, r);
    if !app.camera_input_ready() {
        return Err("Source content mode did not release drawing".into());
    }
    let r = app.camera2d.base.unwrap();
    click(&mut app, &mut time, r);
    if app.camera_input_ready() {
        return Err("Returning to camera view lost input guard".into());
    }
    // Canvas navigation stays available while camera-space painting is blocked.
    app.camera2d.open = false;
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let viewport = app
        .canvas_view
        .viewport_rect()
        .ok_or("Camera canvas missing")?;
    let center = viewport.center();
    let before = app.documents[0].snapshot();
    let history = app.documents[0].history_counts();
    click(
        &mut app,
        &mut time,
        egui::Rect::from_center_size(center, egui::vec2(1., 1.)),
    );
    if app.documents[0].snapshot() != before || app.documents[0].history_counts() != history {
        return Err("Camera-view pointer painted source content".into());
    }
    let zoom = app.documents[0].view.zoom;
    frame(
        &mut app,
        &mut time,
        vec![
            egui::Event::PointerMoved(center),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0., 120.),
                modifiers: Default::default(),
            },
        ],
    );
    if app.documents[0].view.zoom <= zoom {
        return Err("Camera view blocked mouse navigation".into());
    }
    app.begin_camera2d()?;
    for _ in 0..3 {
        frame(&mut app, &mut time, vec![]);
    }
    let r = app.camera2d.keys.unwrap();
    click(&mut app, &mut time, r);
    if !app.keyframes.open || app.keyframes.owner != Owner::Camera {
        return Err("Camera curve editor routing failed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn camera_pointer_controls_source_guard_guides_and_curve_editor() {
        let r = std::env::temp_dir().join(format!("drawer-camera-ui-{}", std::process::id()));
        std::fs::create_dir(&r).unwrap();
        let result = super::smoke(&r);
        std::fs::remove_dir(&r).unwrap();
        result.unwrap();
    }
}
