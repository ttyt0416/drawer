use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    line_tools::Target,
    scene::Affine,
    vector_erase::{self as core, Mode, Settings},
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 32,
            height: 32,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    for (name, points) in [
        ("Edited", vec![[4., 16.], [28., 16.]]),
        ("Reference A", vec![[12., 4.], [12., 28.]]),
        ("Reference B", vec![[20., 4.], [20., 28.]]),
    ] {
        let id = d.scene.lines.add_layer(name)?;
        d.scene
            .lines
            .add_path(id, points, false, 2., [20, 30, 40, 255])?;
    }
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.pressure.device = vec![Some(0.25), None, Some(0.75)];
    p.pressure.controls = vec![0.5, 2., 1.];
    Ok(d)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let original = d.snapshot();
    let target = Some(Target::Vector(layer));
    for mode in [Mode::None, Mode::Whole, Mode::Intersection] {
        let state = core::prepare(
            &d,
            target,
            [16., 16.],
            Settings {
                mode,
                radius: 0.5,
                ..Default::default()
            },
        )?;
        if mode == Mode::None {
            check(state == original, "Do not erase modified a vector")?;
        } else {
            check(
                state.scene.lines.layers[0].paths.is_empty(),
                "Whole/default intersection did not erase a line",
            )?;
            check(
                state.scene.lines.layers[1..] == original.scene.lines.layers[1..],
                "Single-layer erase changed reference layers",
            )?;
        }
    }
    let state = core::prepare(
        &d,
        target,
        [16., 16.],
        Settings {
            refer_all: true,
            radius: 0.5,
            ..Default::default()
        },
    )?;
    check(
        state.scene.lines.layers[0].paths.len() == 2
            && state.scene.lines.layers[1..] == original.scene.lines.layers[1..],
        "Refer all modified references or lost split parts",
    )?;
    let paths = &state.scene.lines.layers[0].paths;
    check(
        paths[0].points == [[4., 16.], [12., 16.]] && paths[1].points == [[20., 16.], [28., 16.]],
        "Intersection endpoints",
    )?;
    for (fragment, (a, b)) in paths.iter().zip([(0., 1. / 3.), (2. / 3., 1.)]) {
        for n in 0..=100 {
            let t = n as f64 / 100.;
            let source = a + (b - a) * t;
            let old = &original.scene.lines.layers[0].paths[0];
            check(
                (fragment.pressure.device_at(t) - old.pressure.device_at(source)).abs() < 1e-9
                    && (fragment.pressure.control_at(t) - old.pressure.control_at(source)).abs()
                        < 1e-9,
                "Fragment changed retained device/edited pressure",
            )?;
        }
    }
    d.commit_state(state)?;
    let after = d.snapshot();
    let file = root.join("vector-erase-pressure.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == after,
        "Fragment native reopen",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&64u32.to_le_bytes());
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Fragment pressure accepted under old header",
    )?;
    check(
        d.undo()
            && d.snapshot() == original
            && d.redo()
            && d.snapshot() == after
            && d.undo()
            && d.snapshot() == original,
        "Split single Undo/Redo",
    )?;
    // Preserve an interior absent device sample as well as both pressure curves.
    let state = core::prepare(
        &d,
        target,
        [24., 16.],
        Settings {
            mode: Mode::Touched,
            radius: 0.5,
            ..Default::default()
        },
    )?;
    check(
        state.scene.lines.layers[0].paths.len() == 2
            && state.scene.lines.layers[0].paths[0]
                .pressure
                .device
                .contains(&None),
        "Touched split lost optional hardware sample",
    )?;
    for (point, radius) in [([28.5, 16.], 0.1), ([3.5, 16.], 0.5)] {
        check(
            core::prepare(
                &d,
                target,
                point,
                Settings {
                    mode: Mode::Whole,
                    radius,
                    ..Default::default()
                },
            )?
            .scene
            .lines
            .layers[0]
                .paths
                .is_empty(),
            "Endpoint cap contact",
        )?;
    }
    check(
        core::prepare(
            &d,
            target,
            [16., 25.],
            Settings {
                mode: Mode::Touched,
                radius: 0.5,
                ..Default::default()
            },
        )? == original,
        "Nearby non-contact geometry erased",
    )?;
    let id = d.scene.lines.layers[0].paths[0].id;
    d.commit_state(core::delete(
        &d,
        layer,
        &crate::vector_edit::Selection {
            paths: vec![id],
            controls: vec![],
        },
    )?)?;
    check(
        d.scene.lines.layers[0].paths.is_empty() && d.undo() && d.snapshot() == original,
        "Selected whole stroke deletion/Undo",
    )?;
    thickness()?;
    nonuniform()?;
    self_cross_and_wrap()?;
    all_layers()?;
    Ok(())
}
fn nonuniform() -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.pressure.device.clear();
    p.pressure.controls = vec![1., 1.];
    d.scene.transform = Affine([4., 0., 0., 0.25, 0., 0.]);
    let before = d.snapshot();
    let settings = Settings {
        mode: Mode::Touched,
        radius: 0.1,
        ..Default::default()
    };
    check(
        core::prepare(&d, Some(Target::Vector(layer)), [64., 5.], settings)? == before,
        "Nonuniform thin axis over-erased blank space",
    )?;
    check(
        core::prepare(&d, Some(Target::Vector(layer)), [64., 4.3], settings)? != before,
        "Nonuniform ellipse contact missed",
    )?;
    d.scene.transform = Affine([1., 0.5, 0.75, 1.5, 2., 3.]);
    let center = d.scene.transform.map([16., 16.]);
    check(
        core::prepare(&d, Some(Target::Vector(layer)), center, settings)? != d.snapshot(),
        "Skewed ellipse contact missed",
    )?;
    // The long segment quadratic must retain a narrow touched interval instead
    // of losing it through discriminant cancellation.
    d.scene.transform = Affine::default();
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[-1e9, 16.], [1e9, 16.]];
    p.pressure = Default::default();
    let state = core::prepare(&d, Some(Target::Vector(layer)), [16., 16.], settings)?;
    check(
        state.scene.lines.layers[0].paths.len() == 2,
        "Long off-page segment contact precision",
    )
}
fn thickness() -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.pressure.device.clear();
    p.pressure.controls = vec![0., 2.];
    let before = d.snapshot();
    let settings = Settings {
        mode: Mode::Touched,
        radius: 0.5,
        ..Default::default()
    };
    check(
        core::prepare(&d, Some(Target::Vector(layer)), [6., 18.], settings)? == before,
        "Thick maximum erased near a thin non-contact section",
    )?;
    check(
        core::prepare(&d, Some(Target::Vector(layer)), [26., 18.], settings)? != before,
        "Thick contact was ignored",
    )?;
    d.scene.lines.layers[0].paths[0].pressure.controls = vec![0., 0.];
    let before = d.snapshot();
    check(
        core::prepare(&d, Some(Target::Vector(layer)), [16., 16.], settings)? == before,
        "Invisible zero pressure was erased",
    )?;
    d.scene.lines.layers[0].locked = true;
    let before = d.snapshot();
    check(
        core::prepare(&d, Some(Target::Vector(layer)), [16., 16.], settings).is_err()
            && d.snapshot() == before,
        "Locked target mutation",
    )
}
fn self_cross_and_wrap() -> Result<(), String> {
    let mut d = fixture()?;
    let id = d.scene.lines.layers[0].id;
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[4., 4.], [28., 28.], [4., 28.], [28., 4.]];
    p.pressure = Default::default();
    let s = core::prepare(
        &d,
        Some(Target::Vector(id)),
        [10., 10.],
        Settings {
            radius: 0.2,
            ..Default::default()
        },
    )?;
    check(
        s.scene.lines.layers[0].paths.len() == 1
            && s.scene.lines.layers[0].paths[0].points[0] == [16., 16.],
        "Self crossing was treated as a vertex or missed",
    )?;
    let p = &mut d.scene.lines.layers[0].paths[0];
    p.points = vec![[4., 4.], [28., 4.], [28., 28.], [4., 28.]];
    p.closed = true;
    p.pressure.device = vec![Some(0.2), Some(0.8), None, Some(0.9)];
    p.pressure.controls = vec![0.5, 1.5, 1., 0.8];
    let state = core::prepare(
        &d,
        Some(Target::Vector(id)),
        [4., 16.],
        Settings {
            mode: Mode::Touched,
            radius: 0.5,
            ..Default::default()
        },
    )?;
    check(
        state.scene.lines.layers[0].paths.len() == 1
            && !state.scene.lines.layers[0].paths[0].closed,
        "Closed wrap fragment topology",
    )?;
    let fragment = &state.scene.lines.layers[0].paths[0];
    let old = &d.scene.lines.layers[0].paths[0];
    for (i, point) in fragment.points.iter().enumerate() {
        let source = if point[0] == 4. && point[1] > 4. && point[1] < 28. {
            1. + (28. - point[1]) / 24. / 3.
        } else if *point == [4., 4.] {
            0.
        } else if *point == [28., 4.] {
            1. / 3.
        } else if *point == [28., 28.] {
            2. / 3.
        } else {
            1.
        };
        let expected = old.pressure_values(source);
        let t = i as f64 / (fragment.points.len() - 1) as f64;
        check(
            (fragment.pressure.device_at(t) - expected.0).abs() < 1e-8
                && (fragment.pressure.control_at(t) - expected.1).abs() < 1e-8,
            &format!(
                "Closed wrap pressure attribution: point={point:?}, source={source}, expected={expected:?}, t={t}, actual={:?}, positions={:?}",
                (
                    fragment.pressure.device_at(t),
                    fragment.pressure.control_at(t)
                ),
                fragment.pressure.device_positions
            ),
        )?;
    }
    // Document transforms affect geometry tests, while a retained fragment uses
    // the same original coordinates and style width.
    d.scene.transform = Affine([2., 0., 0., 2., 3., 5.]);
    let point = d.scene.transform.map([4., 16.]);
    check(
        core::prepare(
            &d,
            Some(Target::Vector(id)),
            point,
            Settings {
                mode: Mode::Touched,
                radius: 1.,
                ..Default::default()
            },
        )?
        .scene
        .lines
        .layers[0]
            .paths
            .len()
            == 1,
        "Transformed closed stroke erase",
    )
}
fn all_layers() -> Result<(), String> {
    let mut d = fixture()?;
    let layer = d.scene.lines.layers[0].id;
    d.scene.lines.layers[1].paths[0].points = vec![[16., 4.], [16., 28.]];
    d.scene.lines.layers[2].paths[0].points = vec![[4., 4.], [28., 28.]];
    let mut samples = vec![crate::color_precision::Sample::default(); 32 * 32];
    samples[16 * 32 + 16] = crate::color_precision::Sample::from8([200, 20, 40, 180]);
    let extent = d.layers[0].extent(d.width, d.height);
    crate::color_storage::set_samples(&mut d.layers[0], &d.color, extent, samples)?;
    let before = d.snapshot();
    let settings = Settings {
        mode: Mode::Whole,
        erase_all: true,
        refer_all: true,
        radius: 0.75,
    };
    let state = core::prepare(&d, Some(Target::Vector(layer)), [16.5, 16.5], settings)?;
    check(
        state.scene.lines.layers.iter().all(|l| l.paths.is_empty())
            && state.layers[0].sample_at(16 * 32 + 16).alpha() == 0.,
        "Erase on all did not erase all touched raster/vector layers",
    )?;
    d.commit_state(state)?;
    check(
        d.undo() && d.snapshot() == before,
        "All-layer erase atomic Undo",
    )?;
    d.scene.lines.layers[1].locked = true;
    let state = core::prepare(&d, Some(Target::Vector(layer)), [16.5, 16.5], settings)?;
    check(
        state.scene.lines.layers[1] == d.scene.lines.layers[1]
            && state.scene.lines.layers[0].paths.is_empty(),
        "All-layer erase failed to retain locked layer",
    )?;
    let state = core::prepare(
        &d,
        Some(Target::Vector(layer)),
        [16.5, 16.5],
        Settings {
            mode: Mode::None,
            ..settings
        },
    )?;
    check(
        state.scene.lines == d.scene.lines && state.layers[0].sample_at(16 * 32 + 16).alpha() == 0.,
        "Do not erase vector mode changed vectors or blocked raster erasure",
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn vector_erase_modes_reference_scope_all_layers_pressure_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
