//! Generated numeric and persistence fixtures, also executed by the installed application.
use crate::{
    bucket,
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    gradient::{self as g, Cover, Repeat, Settings, Shape, VectorFill},
    render::Renderer,
    scene::{Affine, Rect},
    selection::{Mask, Run},
};
fn make(w: u32, h: u32) -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: w,
            height: h,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn assert(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut s = Settings {
        start: [0.5, 0.5],
        end: [4.5, 0.5],
        first: [0, 0, 0, 255],
        last: [200, 100, 40, 255],
        ..Default::default()
    };
    assert(
        s.color([2.5, 0.5], None) == [100, 50, 20, 255],
        "Linear independent midpoint",
    )?;
    for (repeat, left, right) in [
        (Repeat::DoNotRepeat, 0, 200),
        (Repeat::Repeat, 150, 50),
        (Repeat::Reverse, 50, 150),
        (Repeat::NotDraw, 0, 0),
    ] {
        s.repeat = repeat;
        assert(
            s.color([-0.5, 0.5], None)[0] == left && s.color([5.5, 0.5], None)[0] == right,
            "Gradient outside range repeat/reflection",
        )?;
        if repeat == Repeat::NotDraw {
            assert(
                s.color([-0.5, 0.5], None)[3] == 0 && s.color([5.5, 0.5], None)[3] == 0,
                "Not draw alpha",
            )?;
        }
    }
    s.repeat = Repeat::DoNotRepeat;
    let original = s.clone();
    std::mem::swap(&mut s.start, &mut s.end);
    assert(
        s.color([1.5, 0.5], None) == original.color([3.5, 0.5], None),
        "Start/end direction reversal",
    )?;
    s = original;
    for shape in Shape::ALL {
        s.shape = shape;
        s.end = s.start;
        assert(
            s.color([50.0, -80.0], None) == s.first,
            "Zero-length gradient",
        )?;
        s.end = [4.5, 0.5];
    }
    s.shape = Shape::StraightLine;
    s.first = [255, 0, 0, 0];
    s.last = [0, 0, 200, 255];
    assert(
        s.color([2.5, 0.5], None) == [0, 0, 200, 128],
        "Premultiplied transparent stop",
    )?;
    s.first = [0, 0, 0, 255];
    s.last = [200, 100, 40, 255];
    s.shape = Shape::Circle;
    assert(
        s.color([0.5, 2.5], None)[0] == 100,
        "Circle independent radius",
    )?;
    s.shape = Shape::Ellipse;
    s.ellipse_ratio = 0.5;
    assert(s.color([0.5, 1.5], None)[0] == 100, "Ellipse aspect ratio")?;
    s.shape = Shape::Cone;
    for (p, r) in [
        ([4.5, 0.5], 0),
        ([0.5, 4.5], 50),
        ([-3.5, 0.5], 100),
        ([0.5, -3.5], 150),
    ] {
        assert(s.color(p, None)[0] == r, "Cone angular quadrants")?;
    }
    s.shape = Shape::StraightLine;
    s.first = [0, 0, 0, 255];
    s.last = [3, 3, 3, 255];
    s.end = [20.5, 0.5];
    let mut changed = false;
    for x in 0..20 {
        let p = [x as f64 + 0.5, 0.5];
        s.dithering = false;
        let a = s.color(p, None);
        s.dithering = true;
        let b = s.color(p, None);
        changed |= a != b;
        assert(
            a[3] == b[3] && a[0].abs_diff(b[0]) <= 1,
            "Dithering changed alpha or amplitude",
        )?;
    }
    assert(changed, "Dithering had no effect")?;
    let mut d = make(8, 6)?;
    d.layers[0].pixels.fill([40, 50, 60, 255]);
    d.scene.selection_mask = Some(Mask {
        runs: vec![
            Run {
                x: 1,
                y: 2,
                length: 3,
                alpha: 128,
            },
            Run {
                x: 5,
                y: 2,
                length: 1,
                alpha: 255,
            },
        ],
    });
    s = Settings {
        start: [1.5, 2.5],
        end: [5.5, 2.5],
        first: [200, 0, 0, 255],
        last: [0, 0, 200, 255],
        ..Default::default()
    };
    let before = d.snapshot();
    let selection = d.scene.clone();
    let state = g::prepare(&d, 0, s.clone(), true, bucket::Settings::default())?;
    assert(d.snapshot() == before, "Gradient prepare mutated source")?;
    d.commit_state(state)?;
    let renderer = Renderer::new(&d);
    assert(
        renderer.pixel([0.5, 2.5]) == [40, 50, 60, 255]
            && renderer.pixel([4.5, 2.5]) == [40, 50, 60, 255],
        "Gradient outside selection/hole changed",
    )?;
    assert(
        renderer.pixel([1.5, 2.5]) == [120, 25, 30, 255]
            && renderer.pixel([5.5, 2.5]) == [0, 0, 200, 255],
        "Editable gradient selection alpha",
    )?;
    let path = root.join("editable-gradient.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    let mut reopened =
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET).map_err(|e| e.to_string())?;
    assert(
        reopened.snapshot() == d.snapshot(),
        "Gradient native reopen",
    )?;
    let id = d.scene.gradients.layers[0].id;
    s.first = [0, 200, 0, 255];
    let old = d.snapshot();
    d.commit_state(g::edit(&d, id, s.clone())?)?;
    assert(
        Renderer::new(&d).pixel([1.5, 2.5]) == [20, 125, 30, 255],
        "Editable gradient color edit",
    )?;
    assert(
        d.undo() && d.snapshot() == old && d.redo(),
        "Gradient edit Undo/Redo",
    )?;
    reopened.scene.transform = Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    reopened.width = 16;
    reopened.height = 12;
    assert(
        reopened.scene.gradients.layers[0].pixel([1.5, 2.5]) == [200, 0, 0, 128],
        "Gradient creation coordinates changed after page transform",
    )?;
    d.undo();
    d.undo();
    assert(d.snapshot() == before, "Gradient create single Undo")?;
    let raster = g::prepare(
        &d,
        0,
        Settings {
            first: [200, 0, 0, 255],
            ..s.clone()
        },
        false,
        bucket::Settings::default(),
    )?;
    assert(
        raster.scene == selection
            && raster.layers[0].pixels[2 * 8 + 1] == [120, 25, 30, 255]
            && raster.layers[0].pixels[2 * 8 + 4] == before.layers[0].pixels[2 * 8 + 4],
        "Raster gradient selection/scene",
    )?;
    d.layers[0].locked = true;
    assert(
        g::prepare(&d, 0, s.clone(), false, bucket::Settings::default()).is_err(),
        "Locked raster gradient accepted",
    )?;
    d.layers[0].locked = false;
    s.ellipse_ratio = f64::NAN;
    assert(
        g::prepare(&d, 0, s, false, bucket::Settings::default()).is_err(),
        "Invalid gradient accepted",
    )?;
    let mut ring = make(11, 11)?;
    for v in 1..10 {
        for (x, y) in [(v, 1), (v, 9), (1, v), (9, v)] {
            ring.layers[0].pixels[y * 11 + x] = [0, 0, 0, 255];
        }
    }
    ring.layers[0].pixels[5 * 11 + 5] = [80, 30, 160, 255];
    ring.layers[0].pixels[11 + 4] = [0, 0, 0, 128];
    let settings = Settings {
        shape: Shape::Freeform,
        start: [2.5, 2.5],
        end: [8.5, 8.5],
        first: [200, 20, 40, 255],
        last: [20, 40, 200, 255],
        drawing: [20, 200, 40, 255],
        ..Default::default()
    };
    for black in Cover::ALL {
        for transparency in Cover::ALL {
            let s = Settings {
                black,
                transparency,
                ..settings.clone()
            };
            let before = ring.snapshot();
            let state = g::prepare(&ring, 0, s, true, bucket::Settings::default())?;
            ring.commit_state(state)?;
            let r = Renderer::new(&ring);
            assert(
                r.pixel([0.5, 0.5]) == [0; 4] && r.pixel([5.5, 5.5]) == [80, 30, 160, 255],
                "Freeform exterior/colored island changed",
            )?;
            assert(
                r.pixel([2.5, 2.5])
                    == match transparency {
                        Cover::Keep => [0; 4],
                        Cover::Gradient => settings.first,
                        Cover::DrawingColor => settings.drawing,
                    },
                "Freeform transparency policy",
            )?;
            let ink = r.pixel([4.5, 1.5]);
            assert(
                match black {
                    Cover::Keep => ink == [0, 0, 0, 128],
                    Cover::Gradient => ink[3] == 192 && ink[0] > 0,
                    Cover::DrawingColor => ink == [13, 134, 27, 192],
                },
                "Freeform partial black boundary policy",
            )?;
            assert(
                ring.undo() && ring.snapshot() == before,
                "Freeform single Undo",
            )?;
        }
    }
    let excluded = Settings {
        exclude_color: Some([0; 4]),
        ..settings.clone()
    };
    let state = g::prepare(&ring, 0, excluded, false, bucket::Settings::default())?;
    assert(state == ring.snapshot(), "Do not fill this color")?;
    ring.layers[0].pixels[11 + 4] = [0; 4];
    let state = g::prepare(
        &ring,
        0,
        settings.clone(),
        false,
        bucket::Settings::default(),
    )?;
    assert(
        state == ring.snapshot(),
        "Open freeform region leaked outside",
    )?;
    let mut v = make(17, 13)?;
    // Raster enclosure keeps only the middle span adjacent; vector crosses outside both walls.
    for x in 4..13 {
        v.layers[0].pixels[2 * 17 + x] = [0, 0, 0, 255];
        v.layers[0].pixels[10 * 17 + x] = [0, 0, 0, 255];
    }
    for y in 2..11 {
        v.layers[0].pixels[y * 17 + 4] = [0, 0, 0, 255];
        v.layers[0].pixels[y * 17 + 12] = [0, 0, 0, 255];
    }
    let editable = v.scene.lines.add_layer("Gradient boundary")?;
    v.scene.lines.add_path(
        editable,
        vec![[1.5, 6.5], [15.5, 6.5]],
        false,
        1.0,
        [0, 0, 0, 255],
    )?;
    let crossing = v.scene.lines.add_layer("Intersection references")?;
    for x in [4.5, 12.5] {
        v.scene.lines.add_path(
            crossing,
            vec![[x, 0.5], [x, 12.5]],
            false,
            1.0,
            [0, 0, 0, 255],
        )?;
    }
    v.scene.lines.layers[1].locked = true;
    for mode in [
        VectorFill::Keep,
        VectorFill::UpToIntersection,
        VectorFill::WholeLine,
    ] {
        let before = v.snapshot();
        let s = Settings {
            vectors: mode,
            ..settings.clone()
        };
        let state = g::prepare(
            &v,
            0,
            s,
            true,
            bucket::Settings {
                reference: bucket::Reference::Composite,
                ..Default::default()
            },
        )?;
        v.commit_state(state)?;
        let paths = &v.scene.lines.layers[0].paths;
        assert(
            v.scene.lines.layers[1] == before.scene.lines.layers[1],
            "Locked vector reference changed",
        )?;
        assert(
            match mode {
                VectorFill::Keep => paths.len() == 1 && paths[0].hit([8.5, 6.5]),
                VectorFill::UpToIntersection => {
                    paths.iter().any(|p| p.hit([2.5, 6.5]))
                        && paths.iter().any(|p| p.hit([14.5, 6.5]))
                        && !paths.iter().any(|p| p.hit([8.5, 6.5]))
                }
                VectorFill::WholeLine => paths.is_empty(),
            },
            "Gradient vector intersection/whole-line range",
        )?;
        if mode != VectorFill::Keep {
            assert(
                Renderer::new(&v).pixel([8.5, 6.5])[0] > 0,
                "Removed vector span lacks gradient ink",
            )?;
        }
        assert(
            v.undo() && v.snapshot() == before,
            "Gradient vectors atomic Undo",
        )?;
    }
    v.scene.selection = Some(Rect {
        min: [0.0, 0.0],
        max: [17.0, 13.0],
    });
    assert(
        g::prepare(
            &v,
            0,
            Settings {
                vectors: VectorFill::WholeLine,
                ..settings
            },
            true,
            bucket::Settings {
                reference: bucket::Reference::Composite,
                ..Default::default()
            },
        )
        .is_err(),
        "Selected vector replacement accepted",
    )?;
    let mut many = make(512, 512)?;
    for gy in 0..16 {
        for gx in 0..16 {
            let ox = 2 + gx * 8;
            let oy = 2 + gy * 8;
            for v in 0..5 {
                for (x, y) in [
                    (ox + v, oy),
                    (ox + v, oy + 4),
                    (ox, oy + v),
                    (ox + 4, oy + v),
                ] {
                    many.layers[0].pixels[y * 512 + x] = [0, 0, 0, 255];
                }
            }
        }
    }
    assert(
        g::prepare(
            &many,
            0,
            Settings {
                shape: Shape::Freeform,
                ..Default::default()
            },
            true,
            bucket::Settings::default(),
        )
        .is_err(),
        "Gradient region work budget accepted",
    )?;
    let json = serde_json::to_value(crate::scene::Scene::default()).map_err(|e| e.to_string())?;
    assert(
        json.get("gradients").is_none(),
        "Legacy scene gradient omission",
    )?;
    let _: crate::scene::Scene = serde_json::from_value(json).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn numeric_shapes_policies_regions_vectors_undo_reopen() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
