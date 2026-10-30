use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    graph_edit::{self as graph, Channel, Edit, Handle, Preferences},
    keyframes::{self, Curve, Keyframe, Owner, Pose},
};
pub fn fixture() -> Result<(Document, Owner), String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 3,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    d.edit_pixel(0, 1, 1, [210, 30, 40, 255]);
    d.commit_state(crate::timeline_editor::create(&d, 100, Default::default())?)?;
    let owner = Owner::Layer(crate::layer_search::Key::Layer(
        crate::layer_properties::raster(&d, 0),
    ));
    d.commit_state(keyframes::prepare(
        &d,
        owner,
        keyframes::Edit::Enable(true),
    )?)?;
    for f in [0, 10, 20] {
        d.commit_state(keyframes::prepare(
            &d,
            owner,
            keyframes::Edit::Put {
                key: Keyframe {
                    frame: f,
                    pose: Pose {
                        offset: [f as f64, 0.],
                        ..Default::default()
                    },
                    curve: Curve::Linear,
                },
                replace: true,
            },
        )?)?;
    }
    Ok((d, owner))
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let dir = root.join("graph-core-verify");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let check = |b: bool, s: &str| if b { Ok(()) } else { Err(s.to_owned()) };
        let (mut d, owner) = fixture()?;
        let original = d.snapshot();
        let mut p = Preferences {
            snap_time: false,
            ..Default::default()
        };
        graph::prepare(
            &d,
            owner,
            Edit::Handle {
                frame: 10,
                side: Handle::Outgoing,
                point: [12., 12.],
                options: p,
            },
            B,
        )?
        .commit(&mut d)?;
        let t = d.scene.keyframes.get(owner).unwrap();
        check(
            t.keys[0].curve
                == Curve::Bezier {
                    first: [1. / 3.; 2],
                    second: [0.8, 0.8],
                }
                && t.keys[1].curve
                    == Curve::Bezier {
                        first: [0.2, 0.2],
                        second: [2. / 3.; 2],
                    },
            "Joined handle tangents did not mirror",
        )?;
        let prev = t.keys[0].curve;
        p.split_handles = true;
        graph::prepare(
            &d,
            owner,
            Edit::Handle {
                frame: 10,
                side: Handle::Outgoing,
                point: [13., 14.],
                options: p,
            },
            B,
        )?
        .commit(&mut d)?;
        let t = d.scene.keyframes.get(owner).unwrap();
        check(
            t.keys[0].curve == prev
                && matches!(
                    t.keys[1].curve,
                    Curve::Bezier {
                        first: [0.3, 0.4],
                        ..
                    }
                ),
            "Split handle changed the adjacent tangent",
        )?;
        check(
            (graph::sample(t, Channel::OffsetX, 15.).unwrap() - t.at(15).offset[0]).abs() < 1e-12,
            "Graph and rendered interpolation differ",
        )?;
        let saved = d.snapshot();
        d.undo();
        d.undo();
        check(
            d.snapshot() == original,
            "Handle edits did not undo exactly",
        )?;
        d.redo();
        d.redo();
        check(d.snapshot() == saved, "Handle redo lost curve")?;
        let p = Preferences {
            snap_time: true,
            time_step: 2,
            snap_value: true,
            value_step: 2.,
            ..p
        };
        graph::prepare(
            &d,
            owner,
            Edit::Move {
                from: 10,
                point: [13.2, 13.1],
                options: p,
                replace: false,
            },
            B,
        )?
        .commit(&mut d)?;
        check(
            d.scene
                .keyframes
                .get(owner)
                .unwrap()
                .keys
                .iter()
                .any(|k| k.frame == 14 && k.pose.offset[0] == 14.),
            "Graph time/value snap failed",
        )?;
        let s = crate::timeline_editor::prepare(
            &d,
            crate::timeline_editor::Edit::NewTrack("Cels".into()),
            B,
        )?
        .state;
        d.commit_state(s)?;
        d.commit_state(crate::timeline_timing::enable(&d, B)?)?;
        let id = d.scene.timing.clips[0].id;
        crate::clip_edit::prepare(
            &d,
            crate::clip_edit::Edit::Label {
                clip: id,
                first: 1,
                end: 4,
                name: "Range".into(),
            },
            B,
            &std::sync::atomic::AtomicBool::new(false),
        )?
        .commit(&mut d)?;
        let mut s = d.snapshot();
        s.scene.cels.labels.push(crate::cels::Label {
            id: 1,
            name: "Timeline".into(),
            frame: 2,
        });
        s.scene.cels.next_label = 2;
        d.commit_state(s)?;
        let before = d.snapshot();
        graph::prepare(&d, owner, Edit::TimelineLabel { id: 1, frame: 8 }, B)?.commit(&mut d)?;
        let label = d.scene.timing.edit.labels[0].id;
        graph::prepare(
            &d,
            owner,
            Edit::TrackLabel {
                id: label,
                first: 10,
            },
            B,
        )?
        .commit(&mut d)?;
        check(
            d.scene.cels.labels[0].frame == 8
                && d.scene.timing.edit.labels[0].first == 10
                && d.scene.timing.edit.labels[0].end == 13,
            "Label movement lost identity or length",
        )?;
        let after = d.snapshot();
        d.undo();
        d.undo();
        check(d.snapshot() == before, "Label Undo failed")?;
        d.redo();
        d.redo();
        check(d.snapshot() == after, "Label Redo failed")?;
        let path = dir.join("graph.drawer");
        crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
        let loaded = crate::files::open(&path, B).map_err(|e| e.to_string())?;
        check(
            loaded.snapshot() == d.snapshot(),
            "Curve/key/label native reopen differs",
        )?;
        let mut view = Preferences {
            time: [2., 30.],
            value: [-20., 80.],
            ..p
        };
        let before = d.snapshot();
        let history = d.history_counts();
        let dirty = d.dirty;
        view.zoom([2., 0.5], [10., 10.])?;
        view.save(&dir)?;
        check(
            Preferences::load(&dir)? == view
                && d.snapshot() == before
                && d.history_counts() == history
                && d.dirty == dirty,
            "Local graph view changed document or failed to restore",
        )?;
        let settings = dir.join("settings/animation-graph.json");
        std::fs::write(&settings, b"damaged").map_err(|e| e.to_string())?;
        check(
            Preferences::load(&dir).is_err() && std::fs::read(&settings).unwrap() == b"damaged",
            "Invalid graph settings were replaced",
        )?;
        Ok(())
    })();
    let clean = std::fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    result.and(clean)
}
