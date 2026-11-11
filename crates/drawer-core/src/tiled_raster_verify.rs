use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
    layers::{self, Command, Kind},
};
use std::{fs, path::Path};
fn check(b: bool, s: &str) -> Result<(), String> {
    if b { Ok(()) } else { Err(s.into()) }
}
pub fn large(budget: usize, modern: bool) -> Result<Document, String> {
    let spec = CanvasSpec {
        width: 32768,
        height: 32768,
        dpi: 300.,
        background: Background::Transparent,
    };
    if modern {
        Document::create_with_color(
            spec,
            budget,
            crate::color_storage::Config {
                depth: crate::color_precision::Depth::Eight,
                ..Default::default()
            },
        )
    } else {
        Document::create(spec, budget)
    }
    .map_err(|e| e.to_string())
}
pub fn style() -> crate::painting::Style {
    crate::painting::Style {
        clone_stamp: Default::default(),
        variation: Default::default(),
        pattern: Default::default(),
        dynamics: Default::default(),
        sequence: Default::default(),
        filters: Default::default(),
        profile: Default::default(),
        texture: Default::default(),
        material: Default::default(),
        opacity: 1.,
        hardness: 1.,
        spacing: 0.15,
        shape: crate::brush_cursor::Shape {
            size: 3.,
            aspect: 1.,
            angle: 0.,
        },
        tip: crate::assets::BrushTip::Pen,
        color: [30, 60, 90, 255],
        smoothing: 0.,
        pressure: crate::pressure::Mapping {
            size: false,
            opacity: false,
            ..Default::default()
        },
    }
}
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("tiled-raster-fixture");
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut d = large(B, false)?;
        check(
            d.layers[0].tiled.is_some()
                && d.layers[0].pixels.is_empty()
                && d.allocation_bytes() < 65536
                && d.memory_bytes() == 4usize * 1024 * 1024 * 1024,
            "Blank32K canvas allocated its dense4GiB projection",
        )?;
        let points = [(0, 0), (32767, 32767), (16384, 16384)];
        for (n, (x, y)) in points.iter().enumerate() {
            check(
                d.edit_pixel(0, *x, *y, [n as u8 + 20, 80, 120, 255]),
                "Sparse pixel edit rejected",
            )?;
        }
        let picture = d.snapshot();
        let allocated = d.allocation_bytes();
        check(
            allocated < 128 * 1024 && d.resident_bytes() < 256 * 1024,
            "Three sparse pixels exceeded tile/history allocation estimate",
        )?;
        for _ in 0..3 {
            check(d.undo(), "Sparse Undo failed")?;
        }
        check(
            points
                .iter()
                .all(|(x, y)| d.composite_pixel(*x, *y) == [0; 4]),
            "Undo erased/failed sparse pixel semantics",
        )?;
        for _ in 0..3 {
            check(d.redo(), "Sparse Redo failed")?;
        }
        check(
            d.snapshot() == picture,
            "Sparse Redo did not restore exact state",
        )?;
        let file = dir.join("large.drawer");
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        let opened = crate::files::open(&file, B).map_err(|e| e.to_string())?;
        check(
            opened.snapshot() == d.snapshot()
                && opened.allocation_bytes() < 128 * 1024
                && fs::metadata(&file).map_err(|e| e.to_string())?.len() < 200 * 1024,
            "Sparse Native125 packing/roundtrip/budget",
        )?;
        for n in 0..24 {
            let p = layers::prepare(
                &d,
                Command::Create {
                    kind: Kind::Raster,
                    name: format!("Tile layer{n}"),
                    parent: None,
                },
            )?;
            d.commit_state(p.state)?;
        }
        check(
            d.layers.len() == 25
                && d.layers.iter().all(|l| l.tiled.is_some())
                && d.resident_bytes() < 16 * 1024 * 1024,
            "Many sparse layers materialized dense pixels or exceeded cache bound",
        )?;
        check(
            d.edit_pixel(24, 32760, 0, [200, 50, 40, 255]),
            "Sparse upper-layer pixel edit",
        )?;
        let prepared =
            crate::layer_maintenance::prepare(&d, crate::layer_maintenance::Operation::Empty)?;
        check(
            prepared.state.layers.len() == 2
                && prepared
                    .state
                    .layers
                    .iter()
                    .all(|l| l.tiled.as_ref().is_some_and(|t| !t.is_transparent())),
            "Empty-layer cleanup deleted nonempty sparse content",
        )?;
        let mut limited = large(65536, false)?;
        for n in 0..3 {
            check(
                limited.edit_pixel(0, n * 64, 0, [10, 20, 30, 255]),
                "Small-budget sparse setup",
            )?;
        }
        let before = (
            limited.snapshot(),
            limited.history_counts(),
            limited.revision,
            limited.dirty,
        );
        check(
            !limited.edit_pixel(0, 192, 0, [30, 40, 50, 255])
                && limited.take_resource_error().is_some()
                && (
                    limited.snapshot(),
                    limited.history_counts(),
                    limited.revision,
                    limited.dirty,
                ) == before,
            "Sparse OOM changed picture/history or failed to report a budget error",
        )?;
        let mut shared = large(B, false)?;
        shared.set_working_budget(30000);
        check(
            shared.edit_pixel(0, 0, 0, [20, 40, 60, 255]),
            "Shared workspace first tile",
        )?;
        let before = (shared.snapshot(), shared.history_counts(), shared.revision);
        check(
            !shared.edit_pixel(0, 64, 0, [40, 60, 80, 255])
                && shared.take_resource_error().is_some()
                && (shared.snapshot(), shared.history_counts(), shared.revision) == before,
            "Other-document working budget was exceeded",
        )?;
        shared.set_working_budget(65536);
        check(
            shared.edit_pixel(0, 64, 0, [40, 60, 80, 255]) && shared.resident_bytes() <= 65536,
            "Released workspace budget did not resume editing",
        )?;
        let mut modern = large(B, true)?;
        let mut stroke = crate::painting::Stroke::begin(&modern, 0, style())?;
        stroke.sample(&modern, [16384.5, 16384.5])?;
        stroke.sample(&modern, [16392.5, 16384.5])?;
        check(
            stroke.commit(&mut modern)?
                && modern.layers[0]
                    .sample(modern.width, modern.height, 16384, 16384)
                    .rgba8()[3]
                    > 0
                && modern.resident_bytes() < 1024 * 1024,
            "Floating8-bit sparse brush did not paint within bounded tiles",
        )?;
        let painted = modern.snapshot();
        check(
            modern.undo()
                && modern.layers[0]
                    .sample(modern.width, modern.height, 16384, 16384)
                    .rgba8()
                    == [0; 4]
                && modern.redo()
                && modern.snapshot() == painted,
            "Floating sparse stroke Undo/Redo",
        )?;
        let mut dense = Document::create_with_color(
            CanvasSpec {
                width: 256,
                height: 256,
                dpi: 300.,
                background: Background::Transparent,
            },
            B,
            crate::color_storage::Config {
                depth: crate::color_precision::Depth::Eight,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        for n in 0..24 {
            let p = layers::prepare(
                &dense,
                Command::Create {
                    kind: Kind::Raster,
                    name: format!("Dense{n}"),
                    parent: None,
                },
            )?;
            dense.commit_state(p.state)?;
        }
        let latest = dense.snapshot();
        let peak = dense.resident_bytes();
        while dense.undo() {}
        while dense.redo() {}
        check(
            dense.snapshot() == latest && peak <= B && dense.resident_bytes() <= B,
            "Dense layer/repeated Undo memory bound or final picture",
        )?;
        fs::write(
            dir.join("measurements.txt"),
            format!("blank32K<65536;threePixels={allocated};dense25layersUndo={peak};budget={B}\n"),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })();
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn sparse32k_native_cow_mouse_model_many_layers_undo_bounds_and_oom() {
        let root = crate::test_support::TempDir::new();
        super::run(&root.0).unwrap();
    }
}
