use super::*;
use crate::{
    animation::Fps,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B},
    keyframes::{self, Curve, Edit as KeyEdit, Keyframe, Owner, Pose},
    timeline_editor::{self, Edit as CelEdit},
};
fn apply(d: &mut Document, e: Edit) {
    prepare(d, e, B, &AtomicBool::new(false))
        .unwrap()
        .commit(d)
        .unwrap();
}
fn fixture() -> (Document, u64) {
    let mut d = Document::create(
        CanvasSpec {
            width: 3,
            height: 2,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .unwrap();
    d.commit_state(timeline_editor::create(&d, 12, Fps::default()).unwrap())
        .unwrap();
    let p = timeline_editor::prepare(&d, CelEdit::NewTrack("Cels".into()), B).unwrap();
    let track = p.track.unwrap();
    let source = p.source.unwrap();
    d.commit_state(p.state).unwrap();
    let mut child = d.scene.cels.get(source).unwrap().open().unwrap();
    child.edit_pixel(0, 0, 0, [240, 20, 30, 255]);
    let s = crate::cels::replace(&d, source, &child, B).unwrap();
    d.commit_state(s).unwrap();
    d.current_frame = 6;
    let p = timeline_editor::prepare(
        &d,
        CelEdit::NewCel {
            track,
            name: "Blue".into(),
        },
        B,
    )
    .unwrap();
    let source = p.source.unwrap();
    d.commit_state(p.state).unwrap();
    let mut child = d.scene.cels.get(source).unwrap().open().unwrap();
    child.edit_pixel(0, 0, 0, [20, 30, 240, 255]);
    d.commit_state(crate::cels::replace(&d, source, &child, B).unwrap())
        .unwrap();
    d.commit_state(crate::timeline_timing::enable(&d, B).unwrap())
        .unwrap();
    (d, track)
}
fn pixels(d: &Document) -> Vec<Vec<[u8; 4]>> {
    (0..d.animation.as_ref().unwrap().frames)
        .map(|f| {
            let mut n = crate::layer_merge::preview(&d.snapshot(), f).unwrap();
            n.current_frame = f;
            (0..n.height)
                .flat_map(|y| (0..n.width).map(move |x| (x, y)))
                .map(|(x, y)| {
                    crate::float_render::Renderer::new(&n)
                        .unwrap()
                        .display_sample([x as f64 + 0.5, y as f64 + 0.5])
                        .unwrap()
                        .rgba8()
                })
                .collect()
        })
        .collect()
}
#[test]
fn clip_split_join_linked_boundary_labels_folders_bounds_native_and_undo() {
    let (mut d, track) = fixture();
    let id = d.scene.timing.clips[0].id;
    apply(
        &mut d,
        Edit::Label {
            clip: id,
            first: 2,
            end: 9,
            name: "Crossing".into(),
        },
    );
    apply(
        &mut d,
        Edit::Folder {
            name: "Shot".into(),
            children: vec![Node::Clip(id)],
        },
    );
    let folder = d.scene.timing.edit.folders[0].id;
    apply(
        &mut d,
        Edit::Folder {
            name: "Reel".into(),
            children: vec![Node::Folder(folder)],
        },
    );
    let original = d.snapshot();
    let p = pixels(&d);
    apply(&mut d, Edit::Split { id, at: 6 });
    let right = d
        .scene
        .timing
        .clips
        .iter()
        .find(|c| c.first == 6)
        .unwrap()
        .id;
    assert_eq!(
        d.scene
            .timing
            .edit
            .labels
            .iter()
            .map(|l| (l.first, l.end))
            .collect::<Vec<_>>(),
        [(2, 6), (6, 9)]
    );
    assert_eq!(
        d.scene
            .timing
            .edit
            .range(Node::Folder(folder), &d.scene.timing.clips),
        Some((0, 12))
    );
    assert_eq!(pixels(&d), p);
    d.undo();
    assert_eq!(d.snapshot(), original);
    d.redo();
    apply(&mut d, Edit::Join(vec![id, right]));
    assert_eq!(d.scene.timing.clips.len(), 1);
    assert_eq!(pixels(&d), p);
    let before = d.snapshot();
    assert!(prepare(&d, Edit::Split { id, at: 0 }, B, &AtomicBool::new(false)).is_err());
    assert!(prepare(&d, Edit::Join(vec![]), B, &AtomicBool::new(false)).is_err());
    assert!(prepare(&d, Edit::Delete(vec![]), B, &AtomicBool::new(true)).is_err());
    assert!(prepare(&d, Edit::Split { id, at: 3 }, 1, &AtomicBool::new(false)).is_err());
    assert_eq!(d.snapshot(), before);
    assert!(
        prepare(
            &d,
            Edit::Folder {
                name: "Invalid".into(),
                children: vec![Node::Clip(u64::MAX)]
            },
            B,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(
        prepare(
            &d,
            Edit::Split { id, at: 10000 },
            B,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let mut invalid = d.scene.timing.edit.clone();
    invalid.folders[0].children = vec![Node::Folder(invalid.folders[0].id)];
    assert!(!invalid.valid(&d.scene.timing.clips));
    // 256 folders at depth16 with a shared dense subgraph must visit each subtree once.
    let mut graph = Data::default();
    let mut children = vec![Node::Clip(id)];
    for _ in 0..16 {
        let mut next = vec![];
        for _ in 0..16 {
            let folder = graph.id().unwrap();
            graph.folders.push(Folder {
                id: folder,
                name: format!("Folder{folder}"),
                children: children.clone(),
            });
            next.push(Node::Folder(folder));
        }
        children = next;
    }
    assert!(graph.valid(&d.scene.timing.clips));
    assert_eq!(
        graph.range(children[0], &d.scene.timing.clips),
        Some((0, 12))
    );
    assert_eq!(
        graph.leaves(children[0], &d.scene.timing.clips).unwrap(),
        vec![id]
    );
    let temp = crate::test_support::TempDir::new();
    let path = temp.0.join("clips.drawer");
    crate::files::save(&mut d, &path).unwrap();
    let loaded = crate::files::open(&path, B).unwrap();
    assert_eq!(loaded.snapshot(), d.snapshot());
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[8..12].copy_from_slice(&104u32.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    assert!(crate::files::open(&path, B).is_err());
    assert_eq!(track, d.scene.timing.clips[0].track);
}
#[test]
fn move_copy_cut_paste_partial_ranges_labels_off_collisions_and_stale() {
    let (mut d, track) = fixture();
    let id = d.scene.timing.clips[0].id;
    apply(
        &mut d,
        Edit::Label {
            clip: id,
            first: 1,
            end: 5,
            name: "A".into(),
        },
    );
    apply(&mut d, Edit::Split { id, at: 6 });
    let right = d
        .scene
        .timing
        .clips
        .iter()
        .find(|c| c.first == 6)
        .unwrap()
        .id;
    let original = pixels(&d);
    let clip = copy(&d, track, 1, 4).unwrap();
    assert_eq!(clip.bounds(), (1, 4));
    apply(
        &mut d,
        Edit::Cut {
            track,
            first: 1,
            end: 4,
        },
    );
    assert!(pixels(&d)[1][0][3] == 0);
    apply(
        &mut d,
        Edit::Paste {
            clipboard: clip,
            at: 1,
            replace: false,
        },
    );
    assert_eq!(pixels(&d), original);
    let before = d.snapshot();
    assert!(
        prepare(
            &d,
            Edit::Move {
                ids: vec![right],
                at: 0,
                copy: true,
                replace: false
            },
            B,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(d.snapshot(), before);
    let oldlabels = d.scene.timing.edit.labels.clone();
    apply(&mut d, Edit::LinkLabels(false));
    apply(
        &mut d,
        Edit::Move {
            ids: vec![right],
            at: 0,
            copy: true,
            replace: true,
        },
    );
    assert_eq!(
        d.scene
            .timing
            .edit
            .labels
            .iter()
            .map(|l| (&l.name, l.first, l.end))
            .collect::<Vec<_>>(),
        oldlabels
            .iter()
            .map(|l| (&l.name, l.first, l.end))
            .collect::<Vec<_>>()
    );
    let staged = prepare(
        &d,
        Edit::AutoExpand {
            id: right,
            on: true,
        },
        B,
        &AtomicBool::new(false),
    )
    .unwrap();
    d.edit_pixel(0, 2, 1, [1, 2, 3, 255]);
    let before = d.snapshot();
    assert!(staged.commit(&mut d).is_err());
    assert_eq!(d.snapshot(), before);
}
#[test]
fn opted_in_clip_extends_with_key_endpoints_and_folder_aggregates() {
    let (mut d, track) = fixture();
    let id = d.scene.timing.clips[0].id;
    apply(
        &mut d,
        Edit::Bounds {
            id,
            first: 2,
            end: 4,
        },
    );
    apply(&mut d, Edit::AutoExpand { id, on: true });
    assert_eq!(d.scene.timing.clips[0].first, 0);
    assert!(d.scene.timing.clips[0].end >= 7);
    let enabled =
        keyframes::prepare(&d, Owner::Layer(Key::Track(track)), KeyEdit::Enable(true)).unwrap();
    d.commit_state(enabled).unwrap();
    let state = keyframes::prepare(
        &d,
        Owner::Layer(Key::Track(track)),
        KeyEdit::Put {
            key: Keyframe {
                frame: 10,
                pose: Pose::default(),
                curve: Curve::Fixed,
            },
            replace: true,
        },
    )
    .unwrap();
    d.commit_state(state).unwrap();
    assert_eq!(d.scene.timing.clips[0].end, 11);
}
#[test]
fn split_and_merge_tracks_keep_every_rendered_frame_and_native_sources() {
    let (mut d, track) = fixture();
    let before = pixels(&d);
    let sources = d.scene.cels.definitions.clone();
    apply(&mut d, Edit::SplitTrack { track, at: 6 });
    assert_eq!(d.animation.as_ref().unwrap().tracks.len(), 2);
    assert_eq!(pixels(&d), before);
    let to = d.scene.catalog.tracks.id(1);
    apply(
        &mut d,
        Edit::MergeTracks {
            from: to,
            to: track,
            replace: false,
        },
    );
    assert_eq!(d.animation.as_ref().unwrap().tracks.len(), 1);
    assert_eq!(d.scene.cels.definitions, sources);
    assert_eq!(pixels(&d), before);
    d.undo();
    assert_eq!(d.animation.as_ref().unwrap().tracks.len(), 2);
    assert_eq!(pixels(&d), before);
}

#[test]
fn key_collision_replacement_is_atomic_and_copied_keys_stay_enabled() {
    let (mut d, track) = fixture();
    let id = d.scene.timing.clips[0].id;
    apply(&mut d, Edit::Split { id, at: 6 });
    let right = d
        .scene
        .timing
        .clips
        .iter()
        .find(|c| c.first == 6)
        .unwrap()
        .id;
    apply(&mut d, Edit::Delete(vec![right]));
    let owner = Owner::Layer(Key::Track(track));
    d.commit_state(keyframes::prepare(&d, owner, KeyEdit::Enable(true)).unwrap())
        .unwrap();
    for (frame, x) in [(2, 0.25), (8, 0.75)] {
        d.commit_state(
            keyframes::prepare(
                &d,
                owner,
                KeyEdit::Put {
                    key: Keyframe {
                        frame,
                        pose: Pose {
                            offset: [x, 0.],
                            ..Default::default()
                        },
                        curve: Curve::Linear,
                    },
                    replace: true,
                },
            )
            .unwrap(),
        )
        .unwrap();
    }
    let clipboard = copy(&d, track, 0, 3).unwrap();
    let before = d.snapshot();
    let error = prepare(
        &d,
        Edit::Paste {
            clipboard: clipboard.clone(),
            at: 6,
            replace: false,
        },
        B,
        &AtomicBool::new(false),
    )
    .err()
    .unwrap();
    assert!(error.contains("keys collide"), "{error}");
    assert_eq!(d.snapshot(), before);
    apply(
        &mut d,
        Edit::Paste {
            clipboard,
            at: 6,
            replace: true,
        },
    );
    let keys = d.scene.keyframes.get(owner).unwrap();
    assert!(keys.enabled);
    assert_eq!(
        keys.keys.iter().find(|k| k.frame == 8).unwrap().pose.offset[0],
        0.25
    );
    assert!(keys.keys.iter().any(|k| k.frame == 2));
    d.undo();
    assert_eq!(d.snapshot(), before);
}
#[test]
fn animated_track_merge_preserves_an_inactive_arrangement_with_an_empty_new_track() {
    let (mut d, track) = fixture();
    let owner = Owner::Layer(Key::Track(track));
    d.commit_state(keyframes::prepare(&d, owner, KeyEdit::Enable(true)).unwrap())
        .unwrap();
    d.commit_state(
        keyframes::prepare(
            &d,
            owner,
            KeyEdit::Put {
                key: Keyframe {
                    frame: 10,
                    pose: Pose {
                        offset: [0.5, 0.],
                        ..Default::default()
                    },
                    curve: Curve::Linear,
                },
                replace: true,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    d.commit_state(
        crate::timelines::prepare(&d, crate::timelines::Edit::Enable, B, &cancel)
            .unwrap()
            .state,
    )
    .unwrap();
    d.commit_state(
        crate::timelines::prepare(
            &d,
            crate::timelines::Edit::Duplicate {
                name: "Other shot".into(),
                independent: false,
            },
            B,
            &cancel,
        )
        .unwrap()
        .state,
    )
    .unwrap();
    let before = pixels(&d);
    apply(&mut d, Edit::SplitTrack { track, at: 6 });
    let right = d.scene.catalog.tracks.id(1);
    apply(
        &mut d,
        Edit::MergeTracks {
            from: right,
            to: track,
            replace: false,
        },
    );
    assert_eq!(pixels(&d), before);
    d.commit_state(
        crate::timelines::prepare(&d, crate::timelines::Edit::Switch(2), B, &cancel)
            .unwrap()
            .state,
    )
    .unwrap();
    assert_eq!(pixels(&d), before);
    assert_eq!(d.animation.as_ref().unwrap().tracks.len(), 1);
    assert!(crate::timelines::valid_document(&d));
    let temp = crate::test_support::TempDir::new();
    let p = temp.0.join("merged.drawer");
    crate::files::save(&mut d, &p).unwrap();
    let loaded = crate::files::open(&p, B).unwrap();
    assert_eq!(pixels(&loaded), before);
    assert!(crate::timelines::valid_document(&loaded));
}
