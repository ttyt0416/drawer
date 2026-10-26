use super::*;
fn value(ui: &mut egui::Ui, label: &str, v: &mut f64, min: f64, max: f64) {
    ui.add(egui::DragValue::new(v).range(min..=max).prefix(label));
}
fn point(ui: &mut egui::Ui, label: &str, p: &mut [f64; 2]) {
    ui.horizontal(|ui| {
        ui.label(label);
        for v in p {
            value(ui, "", v, -1e6, 1e6);
        }
    });
}
fn rect(ui: &mut egui::Ui, r: &mut Rect) {
    point(ui, "Minimum X/Y", &mut r.min);
    point(ui, "Maximum X/Y", &mut r.max);
}
pub(super) fn object(ui: &mut egui::Ui, d: &Document, e: &mut Draft) {
    ui.add(egui::TextEdit::singleline(&mut e.object.name).char_limit(64));
    ui.collapsing("Object transform", |ui| {
        for (i, label) in [
            "X scale/shear ",
            "Y shear ",
            "X shear ",
            "Y scale ",
            "X position ",
            "Y position ",
        ]
        .into_iter()
        .enumerate()
        {
            value(ui, label, &mut e.object.transform.0[i], -1e6, 1e6);
        }
    });
    match &mut e.object.kind {
        c::Kind::Balloon(b) => {
            rect(ui, &mut b.rect);
            ui.horizontal(|ui| {
                for s in [
                    c::BalloonShape::Ellipse,
                    c::BalloonShape::Rectangle,
                    c::BalloonShape::Cloud,
                ] {
                    ui.radio_value(&mut b.shape, s, format!("{s:?}"));
                }
            });
            ui.horizontal(|ui| {
                for s in [c::Tail::Speech, c::Tail::Thought, c::Tail::None] {
                    ui.radio_value(&mut b.tail, s, format!("{s:?}"));
                }
            });
            point(ui, "Tail tip X/Y", &mut b.tip);
            value(ui, "Tail attachment ° ", &mut b.attachment, -360., 360.);
            value(ui, "Tail width ", &mut b.tail_width, 0., 1e4);
            value(ui, "Border width ", &mut b.width, 0., 128.);
            value(ui, "Text padding ", &mut b.padding, 0., 4096.);
            ui.label("Fill / border");
            ui.horizontal(|ui| {
                ui.color_edit_button_srgba_unmultiplied(&mut b.fill);
                ui.color_edit_button_srgba_unmultiplied(&mut b.ink);
            });
            egui::ComboBox::from_label("Linked text")
                .selected_text(
                    b.text
                        .map_or_else(|| "None".into(), |id| format!("Text#{id}")),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut b.text, None, "None");
                    for (n, o) in d.scene.objects.iter().enumerate() {
                        if let PageObject::Text { text, .. } = o {
                            let id = d.scene.catalog.objects.id(n);
                            ui.selectable_value(
                                &mut b.text,
                                Some(id),
                                format!("#{id} {}", text.chars().take(30).collect::<String>()),
                            );
                        }
                    }
                });
            ui.small("The outline grows to include the linked text, including after object rotation. Text remains independently editable.");
        }
        c::Kind::Frame(f) => {
            ui.label("Polygon points in local coordinates");
            for p in &mut f.polygon {
                point(ui, "", p);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(f.polygon.len() < 128, egui::Button::new("Add corner"))
                    .clicked()
                {
                    let p = *f.polygon.last().unwrap();
                    f.polygon.push([p[0] + 10., p[1] + 10.]);
                }
                if ui
                    .add_enabled(f.polygon.len() > 3, egui::Button::new("Remove last corner"))
                    .clicked()
                {
                    f.polygon.pop();
                }
            });
            value(ui, "Frame gap ", &mut f.gap, 0., 4096.);
            value(ui, "Border width ", &mut f.width, 0., 128.);
            ui.color_edit_button_srgba_unmultiplied(&mut f.ink);
            ui.label("Frame folder contents (clipped inside the frame)");
            if let Ok(rows) = drawer_core::layer_catalog::rows(d) {
                for row in rows {
                    if e.id
                        == match row.key {
                            drawer_core::layer_search::Key::Object(id) => Some(id),
                            _ => None,
                        }
                    {
                        continue;
                    }
                    if !drawer_core::layer_groups::canonical(row.key) {
                        continue;
                    }
                    let mut checked = f.children.contains(&row.key);
                    if ui
                        .checkbox(&mut checked, format!("{} · {:?}", row.name, row.key))
                        .changed()
                    {
                        if checked {
                            f.children.push(row.key);
                        } else {
                            f.children.retain(|k| *k != row.key);
                        }
                    }
                }
            }
        }
        c::Kind::Tone(t) => {
            rect(ui, &mut t.rect);
            egui::ComboBox::from_label("Dot geometry")
                .selected_text(format!("{:?}", t.dot))
                .show_ui(ui, |ui| {
                    for v in c::Dot::ALL {
                        ui.selectable_value(&mut t.dot, v, format!("{v:?}"));
                    }
                });
            value(ui, "Lines per inch ", &mut t.lpi, 1., 4000.);
            value(ui, "Density ", &mut t.density, 0., 1.);
            value(ui, "Angle ° ", &mut t.angle, -360., 360.);
            point(ui, "Phase px", &mut t.phase);
            ui.checkbox(&mut t.gradient, "Density gradient");
            if t.gradient {
                value(ui, "End density ", &mut t.end_density, 0., 1.);
            }
            ui.checkbox(
                &mut t.monochrome,
                "Monochrome tone (luminance / alpha preserved)",
            );
            ui.color_edit_button_srgba_unmultiplied(&mut t.color);
            ui.add(egui::DragValue::new(&mut t.seed).prefix("Fixed seed "));
            if t.lpi > d.dpi / 4. {
                ui.colored_label(egui::Color32::YELLOW,"Fine tone: reduced views can show moiré. Use the prepared output preview before printing.");
            }
        }
        c::Kind::Effect(f) => {
            rect(ui, &mut f.rect);
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.kind, c::Lines::Parallel, "Speed lines");
                ui.radio_value(&mut f.kind, c::Lines::Radial, "Concentration lines");
            });
            point(ui, "Center X/Y", &mut f.center);
            value(ui, "Angle ° ", &mut f.angle, -360., 360.);
            value(ui, "Inner empty radius ", &mut f.inner, 0., 1e6);
            ui.add(
                egui::DragValue::new(&mut f.count)
                    .range(1..=2048)
                    .prefix("Line count "),
            );
            value(ui, "Minimum width ", &mut f.min_width, 0.1, 128.);
            value(ui, "Maximum width ", &mut f.max_width, 0.1, 128.);
            ui.add(egui::DragValue::new(&mut f.seed).prefix("Fixed seed "));
            ui.color_edit_button_srgba_unmultiplied(&mut f.ink);
            ui.small("These editable objects do not enable ruler snapping.");
        }
    }
}
pub(super) fn page(ui: &mut egui::Ui, p: &mut c::PageInfo) {
    ui.label("Work title / author / language");
    ui.add(egui::TextEdit::singleline(&mut p.title).char_limit(256));
    ui.add(egui::TextEdit::singleline(&mut p.author).char_limit(256));
    ui.add(egui::TextEdit::singleline(&mut p.language).char_limit(32));
    ui.horizontal(|ui| {
        for v in &mut p.trim_mm {
            value(ui, "Trim mm (0=canvas) ", v, 0., 1e4);
        }
    });
    value(ui, "Bleed mm ", &mut p.bleed_mm, 0., 1000.);
    value(ui, "Safe margin mm ", &mut p.safe_mm, 0., 1000.);
    ui.checkbox(&mut p.right_binding, "Right binding");
    ui.checkbox(&mut p.first_right, "First page is on the right");
    ui.add(
        egui::DragValue::new(&mut p.number)
            .range(1..=100000)
            .prefix("Page number "),
    );
    ui.add(
        egui::DragValue::new(&mut p.total)
            .range(1..=100000)
            .prefix("Total pages "),
    );
    ui.checkbox(
        &mut p.include_guides,
        "Include trim, bleed and safe guides in output",
    );
    ui.checkbox(&mut p.include_number, "Include page number in output");
}
