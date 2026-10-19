use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    painting::Stroke,
    rulers::{Kind, Runtime},
    scene::{Affine, Point},
    snapping::Settings,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() < 1e-8 && (a[1] - b[1]).abs() < 1e-8
}
fn make() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 64,
            height: 48,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make()?;
    d.scene.grid = crate::aids::Grid {
        spacing: [8.0; 2],
        origin: [0.5; 2],
    };
    d.scene.guides = vec![[[0.0, 10.5], [64.0, 10.5]], [[12.5, 0.0], [12.5, 48.0]]];
    let all = Settings {
        guides: true,
        grid: true,
        ..Default::default()
    };
    let before = d.snapshot();
    let mut r = Runtime::new(&d)?;
    r.configure_snapping(&d, all, 1.0)?;
    check(
        near(r.constrain([9.5, 11.5]), [9.5, 10.5]),
        "Guide did not precede grid",
    )?;
    check(
        near(r.constrain([11.5, 11.5]), [11.5, 10.5]),
        "Equidistant guide order",
    )?;
    check(
        near(r.constrain([25.5, 25.5]), [24.5, 24.5]),
        "Grid fallback",
    )?;
    d.scene.rulers.add(
        "Active",
        Kind::Straight {
            a: [0.0, 6.5],
            b: [64.0, 6.5],
        },
    )?;
    let mut r = Runtime::new(&d)?;
    r.configure_snapping(&d, all, 1.0)?;
    check(
        near(r.constrain([9.5, 11.5]), [9.5, 6.5]),
        "Active ruler did not precede guide/grid",
    )?;
    r.set_snap_disabled(true);
    check(
        near(r.constrain([9.5, 11.5]), [9.5, 11.5]),
        "Temporary snap disable",
    )?;
    r.set_snap_disabled(false);
    check(
        near(r.constrain([9.5, 11.5]), [9.5, 6.5]),
        "Snap restore after modifier release",
    )?;
    let mut off = Runtime::new(&d)?;
    off.configure_snapping(
        &d,
        Settings {
            rulers: false,
            ..all
        },
        1.0,
    )?;
    check(
        near(off.constrain([9.5, 11.5]), [9.5, 10.5]),
        "Disabled ruler suppressed lower-priority guides",
    )?;
    d.scene.rulers = Default::default();
    check(d.snapshot() == before, "Runtime snapping changed page")?;
    d.scene.guides = vec![[[16.5, 0.0], [16.5, 48.0]]];
    for zoom in [0.5, 1.0, 4.0, 16.0] {
        for ppp in [1.0, 2.0] {
            for angle in [0.0, 90.0, 217.0] {
                d.view.zoom = zoom;
                d.view.angle = angle;
                d.view.flipped = true;
                for (screen, hit) in [(4.0 - 1e-6, true), (4.0 + 1e-6, false)] {
                    let mut r = Runtime::new(&d)?;
                    r.configure_snapping(
                        &d,
                        Settings {
                            rulers: false,
                            guides: true,
                            screen_distance: 4.0,
                            ..Default::default()
                        },
                        ppp / zoom,
                    )?;
                    let p = [16.5 + screen * ppp / zoom, 12.5];
                    let (q, used) = r.constrain_with_hit(p);
                    check(
                        used == hit && near(q, if hit { [16.5, 12.5] } else { p }),
                        "Screen capture threshold changed across zoom/DPI/rotation/flip",
                    )?;
                }
            }
        }
    }
    d.scene.transform = Affine([0.0, 2.0, -2.0, 0.0, 50.0, 0.0]);
    d.scene.guides = vec![[[8.5, 0.0], [8.5, 48.0]]];
    let mut r = Runtime::new(&d)?;
    r.configure_snapping(
        &d,
        Settings {
            rulers: false,
            guides: true,
            ..Default::default()
        },
        1.0,
    )?;
    check(
        near(r.constrain([38.0, 18.0]), [38.0, 17.0]),
        "Transformed guide snapping",
    )?;
    let mut r = Runtime::new(&d)?;
    r.configure_snapping(
        &d,
        Settings {
            rulers: false,
            grid: true,
            ..Default::default()
        },
        1.0,
    )?;
    check(
        near(r.constrain([32.0, 18.0]), [33.0, 17.0]),
        "Transformed grid snapping",
    )?;
    let mut direct = make()?;
    direct.scene.rulers.add(
        "Line",
        Kind::Straight {
            a: [0.0, 8.5],
            b: [64.0, 8.5],
        },
    )?;
    let mut style = crate::brush_presets::Preset::capture(
        &crate::workspaces::Snapshot {
            properties: Some(Default::default()),
            ..Default::default()
        },
        crate::assets::BrushTip::PixelPen,
        "Snapping",
    )?
    .style([200, 40, 20, 255]);
    style.shape.size = 1.0;
    style.smoothing = 0.0;
    style.pressure.size = false;
    style.pressure.opacity = false;
    style.profile.minimum_size = 1.0;
    style.profile.minimum_opacity = 1.0;
    let samples = [
        ([4.5, 12.5], false),
        ([12.5, 12.5], true),
        ([20.5, 12.5], false),
    ];
    let before = direct.snapshot();
    let mut stroke = Stroke::begin(&direct, 0, style)?;
    stroke.configure_snapping(&direct, all, 1.0)?;
    for (p, disabled) in samples {
        stroke.set_snap_disabled(disabled);
        stroke.sample(&direct, p)?;
    }
    stroke.commit(&mut direct)?;
    for (x, y, on) in [(4, 8, true), (12, 12, true), (20, 8, true), (20, 12, false)] {
        check(
            (direct.layers[0].pixel(64, 48, x, y)[3] > 0) == on,
            "Direct stroke snap/disable/restore positions",
        )?;
    }
    let mut replay_doc = make()?;
    replay_doc.scene = before.scene.clone();
    let mut replay =
        crate::mouse_pressure::Replay::begin(&replay_doc, 0, style, Default::default())?
            .configure_snapping(all, 1.0)?;
    for (n, (point, disabled)) in samples.into_iter().enumerate() {
        replay.set_snap_disabled(disabled);
        replay.push(crate::mouse_pressure::Sample {
            point,
            micros: Some(n as u64 * 1000),
        })?;
    }
    let predicted = replay.render(&replay_doc, false)?;
    check(
        replay_doc.history_counts() == (0, 0)
            && replay_doc.layers[0].pixels.iter().all(|p| p[3] == 0),
        "Snap replay preview edited page",
    )?;
    drop(predicted);
    replay
        .render(&replay_doc, true)?
        .commit_mouse(&mut replay_doc, replay.metadata())?;
    check(
        replay_doc.layers[0].pixels == direct.layers[0].pixels,
        "Replay modifier transition differs from direct input",
    )?;
    check(
        replay_doc.history_counts().0 == 1
            && replay_doc.undo()
            && replay_doc.layers == before.layers,
        "Snap replay single Undo",
    )?;
    // New shape controls have already resolved grid/ruler priority, so do not snap them again.
    let mut shape = make()?;
    let points = [[5.25, 10.5], [20.25, 14.25]];
    let state = crate::shapes::prepare_resolved(
        &shape,
        crate::shapes::Target::Vector(None),
        &points,
        crate::shapes::Settings {
            kind: crate::shapes::Kind::Polyline,
            snap: true,
            snap_step: 4.0,
            ..Default::default()
        },
    )?;
    shape.commit_state(state)?;
    let def = shape.scene.lines.layers[0].paths[0]
        .definition
        .as_ref()
        .unwrap();
    check(
        def.snapped_controls && def.settings.snap && def.geometry()?.points == points,
        "Resolved shape controls were resnapped by local grid",
    )?;
    let path = root.join("resolved-snap-shape.drawer");
    crate::files::save(&mut shape, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == shape.snapshot(),
        "Snapped shape native reopen",
    )?;
    let mut metadata = make()?;
    let before = metadata.snapshot();
    let mut scene = metadata.scene.clone();
    scene.guides = vec![[[0.0, 4.5], [64.0, 4.5]]];
    scene.grid.spacing = [12.0; 2];
    metadata.edit_scene(scene)?;
    let file = root.join("snap-guide-grid.drawer");
    crate::files::save(&mut metadata, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == metadata.snapshot()
            && metadata.undo()
            && metadata.snapshot() == before,
        "Guide/grid metadata save/one Undo",
    )?;
    let mut snapshot = crate::workspaces::Snapshot {
        properties: Some(Default::default()),
        ..Default::default()
    };
    snapshot.snapping = Settings {
        rulers: false,
        guides: true,
        grid: true,
        screen_distance: 12.5,
        modifier: crate::snapping::Modifier::Alt,
        ..Default::default()
    };
    let file = root.join("snapping-workspace.json");
    snapshot.export(&file).map_err(|e| e.to_string())?;
    check(
        crate::workspaces::Snapshot::read(&file)
            .map_err(|e| e.to_string())?
            .snapping
            == snapshot.snapping,
        "Snap settings reopen",
    )?;
    let mut runtime = Runtime::new(&metadata)?;
    check(
        runtime
            .configure_snapping(
                &metadata,
                Settings {
                    screen_distance: f64::NAN,
                    ..all
                },
                1.0,
            )
            .is_err(),
        "Invalid snap radius accepted",
    )?;
    check(
        runtime.configure_snapping(&metadata, all, 0.0).is_err(),
        "Invalid snapping scale accepted",
    )?;
    let mut small = make()?;
    small.scene.guides = vec![[[0.0, 4.5], [64.0, 4.5]]; 4096];
    let before = small.snapshot();
    let mut stroke = Stroke::with_budget(&small, 0, style, 4096)?;
    check(
        stroke.configure_snapping(&small, all, 1.0).is_err() && small.snapshot() == before,
        "Snapping guide memory limit",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn priority_capture_scale_modifiers_replay_shapes_guides_settings() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
