use crate::{
    deform,
    document::DEFAULT_DOCUMENT_BUDGET,
    mesh_deform::{Mesh, Pin},
    scene::Rect,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn geometry() -> Result<(), String> {
    let area = Rect {
        min: [0.; 2],
        max: [16.; 2],
    };
    let mesh = Mesh::new(3, 3)?;
    check(
        mesh.valid() && Mesh::puppet(3, 3, &[], area)?.identity(),
        "Empty puppet pins changed the mesh",
    )?;
    let mut fixed = Pin {
        vertex: 4,
        target: [0.5; 2],
        degrees: 0.,
        locked: true,
    };
    check(
        Mesh::puppet(3, 3, &[fixed], area)?.identity(),
        "One locked pin changed the mesh",
    )?;
    fixed.locked = false;
    fixed.degrees = 90.;
    let rotated = Mesh::puppet(3, 3, &[fixed], area)?;
    check(
        rotated.valid() && rotated.points[0] == [1., 0.] && rotated.points[8] == [0., 1.],
        "One pin did not rotate a rigid cage by90 degrees",
    )?;
    check(
        rotated.inverse([0.75, 0.25]) == Some([0.25, 0.25]),
        "Puppet inverse90-degree reference",
    )?;
    fixed.degrees = 180.;
    check(
        Mesh::puppet(3, 3, &[fixed], area)?.valid(),
        "Rigid180-degree puppet was falsely rejected as folded",
    )?;
    fixed.degrees = 90.;
    let wide = Mesh::puppet(
        3,
        3,
        &[fixed],
        Rect {
            min: [0.; 2],
            max: [32., 16.],
        },
    )?;
    check(
        wide.valid() && wide.points[0] == [0.75, -0.5],
        "Pin rotation stretched non-square document bounds",
    )?;
    let pins = [
        Pin {
            vertex: 0,
            target: [0.; 2],
            degrees: 0.,
            locked: true,
        },
        Pin {
            vertex: 8,
            target: [1.1, 0.9],
            degrees: 10.,
            locked: false,
        },
    ];
    let moved = Mesh::puppet(3, 3, &pins, area)?;
    check(
        moved.valid()
            && moved.points[0] == [0.; 2]
            && moved.points[8] == [1.1, 0.9]
            && moved.points[4] != [0.5; 2],
        "Multiple pins did not keep anchors and distribute motion",
    )?;
    check(
        Mesh::puppet(3, 3, &[pins[0], pins[0]], area).is_err(),
        "Overlapping pins accepted",
    )?;
    let outside = Pin {
        vertex: 9,
        ..pins[0]
    };
    check(
        Mesh::puppet(3, 3, &[outside], area).is_err(),
        "Out-of-area pin accepted",
    )?;
    let mut folded = mesh.clone();
    folded.points[4] = [2., 2.];
    check(!folded.valid(), "Folded triangle accepted")?;
    let mut duplicate = mesh.clone();
    duplicate.points[4] = duplicate.points[0];
    check(!duplicate.valid(), "Collapsed mesh vertex accepted")?;
    check(
        Mesh::new(1, 3).is_err() && Mesh::new(6, 3).is_err(),
        "Unsupported mesh dimensions accepted",
    )?;
    // A450-degree turn of a rectangular strip overlaps itself without flipping any local triangle.
    let mut wrapped = Mesh::new(5, 2)?;
    for y in 0..2 {
        for x in 0..5 {
            let angle = x as f64 * std::f64::consts::PI * 0.625;
            let radius = if y == 0 { 1. } else { 0.5 };
            wrapped.points[y * 5 + x] = [radius * angle.cos(), radius * angle.sin()];
        }
    }
    check(!wrapped.valid(), "Globally overlapping cage accepted")?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    geometry()?;
    let mut d = crate::deform_verify::fixture()?;
    let original = d.snapshot();
    let counts = d.history_counts();
    let identity = Mesh::new(3, 3)?;
    check(
        deform::prepare(&d, &[0], false, deform::Change::Mesh(&identity))? == original,
        "Identity grid changed storage",
    )?;
    let mut moved = Mesh::new(3, 3)?;
    for p in &mut moved.points {
        p[0] += 0.125;
    }
    let mut faint = crate::deform_verify::fixture()?;
    faint.color = Default::default();
    faint.layers[0].pixels.fill([0; 4]);
    let mut precise = vec![[0; 4]; 256];
    precise[136] = [23456, 12345, 45678, 10];
    faint.layers[0].pixels[136] = crate::color_precision::Sample::from16(precise[136]).rgba8();
    faint.layers[0].precise = Some(precise);
    let state = deform::prepare(&faint, &[0], false, deform::Change::Mesh(&moved))?;
    let at = state.layers[0].pixel_index(16, 16, 10, 8).unwrap();
    check(
        state.layers[0].precise.as_ref().unwrap()[at] == [23456, 12345, 45678, 10]
            && state.layers[0].pixels[at][3] == 0,
        "Mesh lost native16 ink hidden by8-bit alpha preview",
    )?;
    let state = deform::prepare(&d, &[0], false, deform::Change::Mesh(&moved))?;
    for y in 0..16 {
        for x in 0..18 {
            let expected = if x >= 2 {
                d.layers[0].pixel(16, 16, x - 2, y)
            } else {
                [0; 4]
            };
            let actual = state.layers[0].pixel(16, 16, x, y);
            check(
                actual[3] == expected[3] && (actual[3] == 0 || actual == expected),
                "Mesh differs from independent exact translation",
            )?;
        }
    }
    check(
        d.snapshot() == original && d.history_counts() == counts,
        "Mesh preview changed original",
    )?;
    d.commit_state(state)?;
    let changed = d.snapshot();
    let file = root.join("mesh-puppet.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == changed
            && d.history_counts().0 == counts.0 + 1
            && d.undo()
            && d.snapshot() == original
            && d.redo()
            && d.snapshot() == changed,
        "Mesh native/one Undo/Redo failed",
    )?;
    d.undo();
    d.scene.selection = Some(Rect {
        min: [7., 4.],
        max: [10., 12.],
    });
    let mut translated = Mesh::new(3, 3)?;
    for p in &mut translated.points {
        p[0] += 1.;
    }
    let state = deform::prepare(&d, &[0], true, deform::Change::Mesh(&translated))?;
    check(
        state.layers[0].pixel(16, 16, 4, 8) == d.layers[0].pixel(16, 16, 4, 8)
            && state.layers[0].pixel(16, 16, 11, 4) == [255, 0, 0, 128]
            && state.scene.selection_coverage([11.5, 4.5]) > 0,
        "Mesh selected pixels/boundary/following selection",
    )?;
    d.scene.selection = None;
    let mut folded = Mesh::new(3, 3)?;
    folded.points[4] = [-1., -1.];
    let before = d.snapshot();
    check(
        deform::prepare(&d, &[0], false, deform::Change::Mesh(&folded)).is_err()
            && d.snapshot() == before,
        "Rejected fold mutated source",
    )?;
    let pin = Pin {
        vertex: 4,
        target: [0.5; 2],
        degrees: 90.,
        locked: false,
    };
    let rotated = Mesh::puppet(
        3,
        3,
        &[pin],
        Rect {
            min: [0.; 2],
            max: [16.; 2],
        },
    )?;
    let state = deform::prepare(&d, &[0], false, deform::Change::Mesh(&rotated))?;
    check(
        state.layers[0].pixel(16, 16, 11, 8) == [255, 0, 0, 128],
        "Puppet90-degree pixel reference",
    )?;
    let mut budget = crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 256,
            height: 256,
            dpi: 300.,
            background: crate::document::Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    budget.scene.selection = Some(Rect {
        min: [0.; 2],
        max: [256.; 2],
    });
    let mut dense = Mesh::new(5, 5)?;
    dense.points[12][0] += 0.01;
    check(
        deform::prepare(&budget, &[0], true, deform::Change::Mesh(&dense)).is_err(),
        "Mesh selection supersampling exceeded its processing budget",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn pins_zero_one_many_overlap_outside_rotation_and_folds() {
        super::geometry().unwrap();
    }
    #[test]
    fn exact_mesh_pixels_selection_native_atomic_history() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
