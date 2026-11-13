//! Exploratory CPU measurements. No presentation/VRAM/physical latency claims.
use super::editor::check;
use drawer_core::{
    assets::BrushTip,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET as B, Document},
};
use std::{path::Path, time::Instant};
fn baseline(size: u32, layers: usize) -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 1,
            height: 1,
            dpi: 300.,
            background: Background::Transparent,
        },
        B,
    )
    .map_err(|e| e.to_string())?;
    let mut s = d.snapshot();
    s.width = size;
    s.height = size;
    let mut layer = s.layers[0].clone();
    layer.pixels.clear();
    layer.tiled = Some(drawer_core::tiled_raster::Image::new(size, size, [0; 4]));
    s.layers = (0..layers)
        .map(|n| {
            let mut l = layer.clone();
            l.name = format!("CPU baseline {n}");
            l
        })
        .collect();
    s.scene.layer_ids.initialize(layers)?;
    d.commit_state(s)?;
    d.clear_session_history();
    Ok(d)
}
fn paint(d: &mut Document, long: bool, color: [u8; 4]) -> Result<(f64, usize), String> {
    let mut style = drawer_core::selection_paint::style(BrushTip::Pen)?;
    style.shape.size = 100.;
    style.profile.image = Default::default();
    style.opacity = 1.;
    style.spacing = 0.2;
    style.color = color;
    let start = Instant::now();
    let mut stroke = drawer_core::painting::Stroke::begin(d, 0, style)?;
    let count = if long { 65 } else { 2 };
    for n in 0..count {
        stroke.sample(d, [128.5 + n as f64 * 16., 256.5])?;
    }
    stroke.finish_material(d, || false)?;
    let pixels = stroke.changed_pixels();
    check(stroke.commit(d)?, "CPU benchmark stroke made no change")?;
    Ok((start.elapsed().as_secs_f64() * 1000., pixels))
}
fn metric(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    let index = |q: f64| {
        ((values.len() as f64 * q).ceil() as usize)
            .saturating_sub(1)
            .min(values.len() - 1)
    };
    serde_json::json!({"samples":values.len(),"p50ms":values[index(0.5)],"p95ms":values[index(0.95)],"minimumMs":values[0],"maximumMs":values[values.len()-1]})
}
fn disk(root: &Path) -> Result<u64, String> {
    let mut bytes = 0;
    for e in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let m = e.metadata().map_err(|e| e.to_string())?;
        if m.is_file() {
            bytes += m.len();
        }
    }
    Ok(bytes)
}
fn render(d: &Document, zoom: f64, angle: f64, center: [f64; 2]) -> Result<f64, String> {
    let start = Instant::now();
    let mut view = drawer_core::view::View {
        zoom,
        angle,
        ..Default::default()
    };
    let size = [f64::from(d.width), f64::from(d.height)];
    view.center_on(center, size, [0.; 2], 1.);
    let r = drawer_core::render::Renderer::new(d);
    let mut alpha = 0u64;
    for y in 0..32 {
        for x in 0..32 {
            let p = view.screen_to_document(
                [f64::from(x) - 15.5, f64::from(y) - 15.5],
                size,
                [0.; 2],
                1.,
            );
            alpha += u64::from(r.sample(p)?.rgba8()[3]);
        }
    }
    std::hint::black_box(alpha);
    Ok(start.elapsed().as_secs_f64() * 1000.)
}
pub(super) fn run(root: &Path) -> Result<serde_json::Value, String> {
    let mut cases = vec![];
    for (size, layers) in [(4096, 20), (8192, 100)] {
        let dir = root.join(format!("cpu-{size}-{layers}"));
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        let mut d = baseline(size, layers)?;
        let mut short = vec![];
        let mut long = vec![];
        let mut undo = vec![];
        let mut redo = vec![];
        let mut views = [vec![], vec![], vec![]];
        let mut peak = d.resident_bytes();
        let mut active = 0;
        for is_long in [false, true] {
            for _ in 0..3 {
                let before = d.snapshot();
                let (ms, changed) = paint(&mut d, is_long, [80, 100, 200, 255])?;
                active = active.max(changed);
                if is_long {
                    long.push(ms);
                } else {
                    short.push(ms);
                }
                peak = peak.max(d.resident_bytes());
                let center = if is_long {
                    [512.5, 256.5]
                } else {
                    [136.5, 256.5]
                };
                for (n, (zoom, angle)) in [(1., 0.), (0.125, 0.), (1., 37.)].into_iter().enumerate()
                {
                    views[n].push(render(&d, zoom, angle, center)?);
                }
                let after = d.snapshot();
                let start = Instant::now();
                check(d.undo(), "CPU benchmark Undo failed")?;
                undo.push(start.elapsed().as_secs_f64() * 1000.);
                check(
                    d.snapshot() == before,
                    "CPU benchmark Undo changed baseline",
                )?;
                let start = Instant::now();
                check(d.redo(), "CPU benchmark Redo failed")?;
                redo.push(start.elapsed().as_secs_f64() * 1000.);
                check(
                    d.snapshot() == after && d.undo() && d.snapshot() == before,
                    "CPU benchmark Redo state differs",
                )?;
            }
        }
        paint(&mut d, true, [80, 100, 200, 255])?;
        let mut frozen = d.current_copy();
        let expected = frozen.snapshot();
        let file = dir.join("cpu.drawer");
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (go_tx, go_rx) = std::sync::mpsc::sync_channel(1);
        let save_dir = dir.clone();
        let save_file = file.clone();
        let save_start = Instant::now();
        let worker = std::thread::spawn(move || -> Result<u64, String> {
            let mut peak = 0;
            drawer_core::files::save_observed(&mut frozen, &save_file, &mut |stage| {
                if stage == drawer_core::atomic_file::Stage::Created {
                    ready_tx.send(()).map_err(std::io::Error::other)?;
                    go_rx.recv().map_err(std::io::Error::other)?;
                }
                peak = peak.max(disk(&save_dir).map_err(std::io::Error::other)?);
                Ok(())
            })
            .map_err(|e| e.to_string())?;
            Ok(peak)
        });
        let editing = match ready_rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(()) => paint(&mut d, false, [200, 80, 40, 255]),
            Err(e) => Err(e.to_string()),
        };
        let _ = go_tx.send(());
        let disk_peak = worker.join().map_err(|_| "CPU save worker panicked")??;
        let during = editing?;
        let save_ms = save_start.elapsed().as_secs_f64() * 1000.;
        check(
            drawer_core::files::open(&file, B)
                .map_err(|e| e.to_string())?
                .snapshot()
                == expected
                && d.snapshot() != expected,
            "Concurrent CPU edit changed frozen save contents",
        )?;
        peak = peak.max(d.resident_bytes());
        cases.push(serde_json::json!({"size":[size,size],"layers":layers,"storage":"RGBA8 sparse64 tiles; valid generated document","denseEquivalentBytes":u64::from(size)*u64::from(size)*4*layers as u64,
            "blankOtherLayers":layers-1,"penSizeDocumentPixels":100,"maximumChangedPixelsPerStroke":active,"short2Samples":metric(short),"long65Samples":metric(long),"undo":metric(undo),"redo":metric(redo),
            "cpuRender32x32":{"normal":metric(std::mem::take(&mut views[0])),"zoom0_125":metric(std::mem::take(&mut views[1])),"rotation37":metric(std::mem::take(&mut views[2]))},
            "documentAndHistoryResidentEstimatePeakBytes":peak,"nativeSaveIncludingStagePauseMs":save_ms,"liveCpuStrokeWhileFrozenSavePausedMs":during.0,"frozenSaveExact":true,"observedSaveDirectoryPeakBytes":disk_peak}));
    }
    Ok(
        serde_json::json!({"criterionComplete":false,"scope":"exploratory CPU engine measurements; three short/three long strokes per document,32x32 render probes","defaultDocumentBudgetBytes":B,"documents":cases,
        "inputToDisplayP95ms":null,"windowFrameP95ms":null,"processRamPeakBytes":null,"vramUsageBytes":null,"longDurationSessionValidated":false,"displayOrGpuContextCreated":false,
        "pending":["actual event-to-present/display latency","full application frame times","process RAM and GPU VRAM under the complete workload","long-duration baseline runs","reference GPU/driver and display conditions"]}),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn large_logical_baselines_use_bounded_sparse_storage() {
        for (size, layers) in [(4096, 20), (8192, 100)] {
            let d = super::baseline(size, layers).unwrap();
            assert_eq!((d.width, d.height, d.layers.len()), (size, size, layers));
            assert!(
                d.layers
                    .iter()
                    .all(|l| l.pixels.is_empty() && l.tiled.is_some())
            );
            assert!(d.resident_bytes() < 1024 * 1024);
        }
    }
}
