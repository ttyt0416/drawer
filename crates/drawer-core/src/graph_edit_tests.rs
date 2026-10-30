use super::*;
use crate::document::DEFAULT_DOCUMENT_BUDGET as B;
#[test]
fn joined_split_handles_snap_labels_undo_native_and_view_restart() {
    let temp = crate::test_support::TempDir::new();
    crate::graph_verify::verify(&temp.0).unwrap();
}
#[test]
fn scale_time_value_collisions_limits_stale_and_cancel_preserve_sources() {
    let (mut d, owner) = crate::graph_verify::fixture().unwrap();
    let original = d.snapshot();
    let e = || Edit::Scale {
        first: 0,
        last: 20,
        time: [2, 1],
        pivot: 0,
        value: 2.,
        value_pivot: 0.,
        channel: Channel::OffsetX,
        replace: false,
    };
    let p = prepare(&d, owner, e(), B).unwrap();
    assert_eq!(d.snapshot(), original);
    p.commit(&mut d).unwrap();
    let keys = &d.scene.keyframes.get(owner).unwrap().keys;
    assert_eq!(
        keys.iter()
            .map(|k| (k.frame, k.pose.offset[0]))
            .collect::<Vec<_>>(),
        [(0, 0.), (20, 20.), (40, 40.)]
    );
    assert_eq!(d.layers, original.layers);
    d.undo();
    assert_eq!(d.snapshot(), original);
    for edit in [
        Edit::Scale {
            first: 1,
            last: 1,
            time: [1, 1],
            pivot: 0,
            value: 1.,
            value_pivot: 0.,
            channel: Channel::OffsetX,
            replace: false,
        },
        Edit::Scale {
            first: 0,
            last: 20,
            time: [1000, 1],
            pivot: 0,
            value: 1.,
            value_pivot: 0.,
            channel: Channel::OffsetX,
            replace: false,
        },
        Edit::Scale {
            first: 0,
            last: 20,
            time: [1, 1000],
            pivot: 0,
            value: 1.,
            value_pivot: 0.,
            channel: Channel::OffsetX,
            replace: false,
        },
        Edit::Move {
            from: 10,
            point: [20., 50.],
            options: Default::default(),
            replace: false,
        },
        Edit::Move {
            from: 10,
            point: [99., 2.],
            options: Preferences {
                channel: Channel::Opacity,
                ..Default::default()
            },
            replace: false,
        },
    ] {
        assert!(prepare(&d, owner, edit, B).is_err());
        assert_eq!(d.snapshot(), original);
    }
    prepare(
        &d,
        owner,
        Edit::Scale {
            first: 0,
            last: 20,
            time: [1, 1000],
            pivot: 0,
            value: 1.,
            value_pivot: 0.,
            channel: Channel::OffsetX,
            replace: true,
        },
        B,
    )
    .unwrap()
    .commit(&mut d)
    .unwrap();
    assert_eq!(d.scene.keyframes.get(owner).unwrap().keys.len(), 1);
    assert_eq!(
        d.scene.keyframes.get(owner).unwrap().keys[0].pose.offset[0],
        20.
    );
    d.undo();
    let stale = prepare(&d, owner, e(), B).unwrap();
    d.commit_state(
        crate::keyframes::prepare(&d, owner, crate::keyframes::Edit::Move { from: 10, to: 11 })
            .unwrap(),
    )
    .unwrap();
    let before = d.snapshot();
    assert!(stale.commit(&mut d).is_err());
    assert_eq!(d.snapshot(), before);
    assert!(prepare(&d, owner, e(), 1).is_err());
}
#[test]
fn axes_zoom_extremes_and_endpoint_handle_errors() {
    let p = Preferences {
        snap_time: true,
        time_step: 2,
        snap_value: true,
        value_step: 0.5,
        ..Default::default()
    };
    assert_eq!(p.snap([10., 10.], [13.2, 12.7]).unwrap(), [14., 12.5]);
    assert_eq!(
        Preferences {
            axis: Axis::Time,
            ..p
        }
        .snap([10., 10.], [13.2, 12.7])
        .unwrap(),
        [14., 10.]
    );
    assert_eq!(
        Preferences {
            axis: Axis::Value,
            ..p
        }
        .snap([10., 10.], [13.2, 12.7])
        .unwrap(),
        [10., 12.5]
    );
    assert!(p.snap([0., 0.], [f64::NAN, 0.]).is_err());
    let mut p = p;
    let old = p;
    assert!(p.zoom([0., 1.], [0., 0.]).is_err());
    assert_eq!(p, old);
    assert!(
        !Preferences {
            time: [0., 0.],
            ..p
        }
        .valid()
    );
    assert!(
        !Preferences {
            value_step: 0.,
            ..p
        }
        .valid()
    );
    let (mut d, owner) = crate::graph_verify::fixture().unwrap();
    assert!(
        prepare(
            &d,
            owner,
            Edit::Handle {
                frame: 0,
                side: Handle::Incoming,
                point: [0., 0.],
                options: p
            },
            B
        )
        .is_err()
    );
    assert!(
        prepare(
            &d,
            owner,
            Edit::Handle {
                frame: 20,
                side: Handle::Outgoing,
                point: [21., 21.],
                options: p
            },
            B
        )
        .is_err()
    );
    assert!(
        prepare(
            &d,
            owner,
            Edit::TimelineLabel {
                id: 1,
                frame: 10000
            },
            B
        )
        .is_err()
    );
    let mut state = d.snapshot();
    state.animation.as_mut().unwrap().frames = 10000;
    d.commit_state(state).unwrap();
    prepare(
        &d,
        owner,
        Edit::Move {
            from: 20,
            point: [9999., 1e9],
            options: Preferences::default(),
            replace: false,
        },
        B,
    )
    .unwrap()
    .commit(&mut d)
    .unwrap();
    assert!(
        d.scene
            .keyframes
            .get(owner)
            .unwrap()
            .keys
            .iter()
            .any(|k| k.frame == 9999 && k.pose.offset[0] == 1e9)
    );
    let before = d.snapshot();
    assert!(
        prepare(
            &d,
            owner,
            Edit::Move {
                from: 9999,
                point: [10000., 1e9],
                options: Preferences::default(),
                replace: false
            },
            B
        )
        .is_err()
    );
    assert_eq!(d.snapshot(), before);
}
