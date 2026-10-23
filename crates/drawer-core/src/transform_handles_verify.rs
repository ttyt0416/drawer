use crate::{
    document::DEFAULT_DOCUMENT_BUDGET, scene::Rect, transform_handles::*, vector_scope::Transform,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-8)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let bounds = Rect {
        min: [-2., -1.],
        max: [2., 1.],
    };
    let t = Transform::default();
    let q = corners(bounds);
    check(
        pick(bounds, t, [0., 0.], 0.1)? == Handle::Move
            && pick(bounds, t, [3., 0.], 0.1)? == Handle::Rotate,
        "Inside/outside handle classes",
    )?;
    for n in 0..4 {
        check(
            pick(bounds, t, q[n], 0.1)? == Handle::Corner(n),
            "Corner priority",
        )?;
        let anchor = q[(n + 2) % 4];
        let p = [0, 1].map(|k| anchor[k] + (q[n][k] - anchor[k]) * 2.);
        let changed = Drag {
            handle: Handle::Corner(n),
            bounds,
            source: t,
            start: q[n],
        }
        .update(p, true, Snap::default())?;
        check(
            close(changed.scale, [2., 2.]) && close(changed.matrix()?.map(anchor), anchor),
            "Aspect corner scaling must retain opposite anchor",
        )?;
    }
    let edge = Drag {
        handle: Handle::Edge(1),
        bounds,
        source: t,
        start: [2., 0.],
    }
    .update(
        [4., 0.],
        true,
        Snap {
            enabled: true,
            degrees: 15,
        },
    )?;
    check(
        close(edge.scale, [1.5, 1.]),
        "Side handles scale one axis even with aspect lock and angle snapping",
    )?;
    let unlocked = Drag {
        handle: Handle::Corner(2),
        bounds,
        source: t,
        start: q[2],
    }
    .update([3., 2.], false, Snap::default())?;
    check(
        close(unlocked.scale, [1.25, 1.5]),
        "Unlocked corner scaling is independent",
    )?;
    let moved = Drag {
        handle: Handle::Move,
        bounds,
        source: t,
        start: [0., 0.],
    }
    .update([3., 4.], true, Snap::default())?;
    check(
        close(moved.translation, [3., 4.]),
        "Interior handle movement",
    )?;
    let rotate = Drag {
        handle: Handle::Rotate,
        bounds,
        source: t,
        start: [3., 0.],
    };
    let snapped = rotate.update(
        [0., 3.],
        false,
        Snap {
            enabled: true,
            degrees: 15,
        },
    )?;
    check(
        (snapped.degrees - 90.).abs() < 1e-9,
        "Outside handle rotation",
    )?;
    let snap = Snap {
        enabled: true,
        degrees: 15,
    };
    for (v, expected) in [
        (7.5 - 1e-7, 0.),
        (7.5 + 1e-7, 15.),
        (-7.5 + 1e-7, 0.),
        (-7.5 - 1e-7, 345.),
        (360., 0.),
    ] {
        check(
            (snap.angle(v)? - expected).abs() < 1e-9,
            "Independent nearest angle boundary/360",
        )?;
    }
    check(
        Snap {
            enabled: true,
            degrees: 0,
        }
        .angle(10.)
        .is_err()
            && Snap {
                enabled: true,
                degrees: 91,
            }
            .angle(10.)
            .is_err()
            && snap.angle(f64::NAN).is_err(),
        "Invalid angle settings",
    )?;
    check(
        Drag {
            handle: Handle::Corner(2),
            bounds,
            source: t,
            start: q[2],
        }
        .update(q[0], true, snap)
        .is_err(),
        "Zero scale rejected",
    )?;
    let flipped = Drag {
        handle: Handle::Corner(2),
        bounds,
        source: t,
        start: q[2],
    }
    .update([-6., -3.], true, snap)?;
    check(close(flipped.scale, [-1., -1.]), "Negative aspect scale")?;
    for size in [1., 1e-8] {
        let r = Rect {
            min: [0., 0.],
            max: [size, size],
        };
        let out = Drag {
            handle: Handle::Corner(2),
            bounds: r,
            source: t,
            start: r.max,
        }
        .update([size * 2.; 2], true, snap)?;
        check(close(out.scale, [2.; 2]), "1px/tiny transform box")?;
    }
    let mut d = crate::affine_transform_verify::fixture()?;
    let original = d.snapshot();
    let transform = Transform {
        degrees: 30.,
        translation: [2., 1.],
        pivot: [1.; 2],
        ..Default::default()
    };
    let result = crate::affine_transform::prepare(
        &d,
        crate::layer_search::Key::Layer(crate::layer_properties::Target::Raster(1)),
        false,
        transform,
        false,
        crate::interpolation::Filter::Nearest,
    )?;
    check(
        matches!(
            result.state.scene.last_transform,
            Some(Record::Affine { degrees: 30., .. })
        ),
        "Applied angle/matrix receipt missing",
    )?;
    let count = d.history_counts().0;
    d.commit_state(result.state)?;
    check(
        d.history_counts().0 == count + 1,
        "Transform receipt is part of one Undo",
    )?;
    let path = root.join("transform-handles.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Angle/matrix native mismatch",
    )?;
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&79u32.to_le_bytes());
    let old = root.join("transform-handles-old.drawer");
    std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&old, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted transform receipt",
    )?;
    check(
        d.undo() && d.snapshot() == original,
        "Transform receipt Undo mismatch",
    )?;
    let tiny = Transform {
        scale: [1e-6; 2],
        ..Default::default()
    };
    check(
        Record::Affine {
            degrees: 0.,
            matrix: tiny.matrix()?,
        }
        .valid(),
        "A valid tiny transform must have a serializable receipt",
    )?;
    let big = crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 128,
            height: 128,
            dpi: 300.,
            background: crate::document::Background::Color([30, 50, 70]),
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let before = big.snapshot();
    let warp = crate::material_mapping::Warp::Distort([[0., 0.], [64., 0.], [64., 64.], [0., 64.]]);
    check(
        crate::deform::prepare(&big, &[0], false, crate::deform::Change::Quad(&warp)).is_err()
            && big.snapshot() == before,
        "Oversized quad output must fail during preview without changing source",
    )?;
    let rotated = Transform {
        degrees: 37.,
        scale: [-1.2, 0.8],
        pivot: [2., 3.],
        translation: [-5., 2.],
        ..Default::default()
    };
    let matrix = rotated.matrix()?;
    let anchor = q[0];
    let corner = q[2];
    let start = matrix.map(corner);
    let fixed = matrix.map(anchor);
    let desired = [0, 1].map(|n| fixed[n] + (start[n] - fixed[n]) * 1.5);
    let result = Drag {
        handle: Handle::Corner(2),
        bounds,
        source: rotated,
        start,
    }
    .update(desired, true, Snap::default())?;
    check(
        close(result.matrix()?.map(anchor), fixed) && close(result.scale, [-1.8, 1.2]),
        "Rotated/reflected corner anchor",
    )?;
    let workspace = crate::workspaces::Snapshot {
        transform_snap: snap,
        ..Default::default()
    };
    let encoded = serde_json::to_vec(&workspace).map_err(|e| e.to_string())?;
    check(
        serde_json::from_slice::<crate::workspaces::Snapshot>(&encoded)
            .map_err(|e| e.to_string())?
            .transform_snap
            == snap,
        "Snap local serialization",
    )?;
    for points in [
        [[0., 0.], [1., 1.], [1., 0.], [0., 1.]],
        [[0., 0.], [1., 0.], [0.5, 0.25], [0., 1.]],
        [[0., 0.], [1., 0.], [2., 0.], [3., 0.]],
    ] {
        check(
            !crate::material_mapping::Warp::Distort(points).valid()
                && !crate::material_mapping::Warp::Perspective(points).valid(),
            "Concave/crossed/zero area quadrilateral",
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn transform_handles_anchors_snap_tiny_limits_receipts_native_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
