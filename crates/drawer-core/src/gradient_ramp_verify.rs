use crate::{
    gradient::{self as g, Repeat, Settings},
    gradient_ramp::{Brightness, Curve, History, Library, Ramp, Space, Stop},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn luminance(c: [f64; 4]) -> f64 {
    c[0..3]
        .iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(v, w)| crate::color_mix::linear(v / 255.0) * w)
        .sum()
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let a = [255, 0, 0, 255];
    let b = [0, 0, 255, 255];
    let mut ramp = Ramp::two(a, b);
    check(
        ramp.valid()
            && ramp.sample(0.0) == a.map(f64::from)
            && ramp.sample(1.0) == b.map(f64::from),
        "Ramp endpoints",
    )?;
    let mut colors = vec![];
    for space in Space::ALL {
        ramp.space = space;
        let c = ramp.sample(0.5).map(|v| v.round() as u8);
        match space {
            Space::Standard => check(c == [128, 0, 128, 255], "Standard primary midpoint")?,
            Space::LinearRgb => check(c == [188, 0, 188, 255], "Linear RGB midpoint")?,
            Space::PerceptualA => check(
                c[0].abs_diff(140) <= 1 && c[1].abs_diff(83) <= 1 && c[2].abs_diff(162) <= 1,
                "Oklab primary midpoint",
            )?,
            Space::PerceptualB => check(
                c[0].abs_diff(202) <= 1 && c[1] == 0 && c[2].abs_diff(136) <= 1,
                "CIELAB primary midpoint",
            )?,
        }
        colors.push(c);
    }
    colors.sort();
    colors.dedup();
    check(colors.len() == 4, "Gradient spaces collapsed")?;
    ramp.space = Space::Standard;
    let mut previous = 0.0;
    for correction in Brightness::ALL {
        ramp.brightness = correction;
        let c = ramp.sample(0.5);
        let y = luminance(c);
        check(y > previous, "Brightness levels are not distinct/monotonic")?;
        previous = y;
        check(
            c[3] == 255.0 && ramp.sample(0.0) == a.map(f64::from),
            "Brightness changed alpha/endpoints",
        )?;
    }
    check(
        (previous - 0.1424).abs() < 1e-6,
        "Highest correction independent luminance",
    )?;
    for space in Space::ALL {
        for brightness in Brightness::ALL {
            ramp = Ramp::two(a, [0, 255, 0, 0]);
            ramp.space = space;
            ramp.brightness = brightness;
            let c = ramp.sample(0.5).map(|v| v.round() as u8);
            check(
                c == [255, 0, 0, 128],
                "Transparent stop hidden RGB polluted ramp",
            )?;
        }
    }
    ramp = Ramp::two(a, b);
    ramp.stops.insert(1, Stop::new(0.5, [0, 255, 0, 255]));
    ramp.stops.insert(2, Stop::new(0.5, [255, 255, 0, 128]));
    check(
        ramp.valid()
            && ramp.sample(0.5) == [255.0, 255.0, 0.0, 128.0]
            && ramp.sample(0.5 - 1e-8)[1] > 254.0,
        "Equal-position stop hard edge",
    )?;
    ramp = Ramp::two([0, 0, 0, 255], [200, 100, 40, 255]);
    for (curve, value) in [
        (Curve::Linear, 50.0),
        (Curve::Smooth, 31.25),
        (Curve::EaseIn, 12.5),
        (Curve::EaseOut, 87.5),
    ] {
        ramp.stops[0].curve = curve;
        check(
            (ramp.sample(0.25)[0] - value).abs() < 1e-9,
            "Independent node curve",
        )?;
    }
    ramp.stops[0].curve = Curve::Linear;
    ramp.stops[0].midpoint = 0.25;
    check((ramp.sample(0.25)[0] - 100.0).abs() < 1e-9, "Node midpoint")?;
    let good = ramp.clone();
    ramp.stops.pop();
    check(!ramp.valid(), "One-stop ramp accepted")?;
    ramp = good.clone();
    ramp.stops[0].position = -0.001;
    check(!ramp.valid(), "Out-of-range stop accepted")?;
    ramp = good.clone();
    ramp.stops[1].position = f64::NAN;
    check(!ramp.valid(), "NaN stop accepted")?;
    check(
        serde_json::from_str::<Stop>(
            r#"{"position":0,"color":[0,0,0,256],"midpoint":0.5,"curve":"Linear"}"#,
        )
        .is_err(),
        "Out-of-range alpha accepted",
    )?;
    let settings = Settings {
        ramp: Some(good.clone()),
        repeat: Repeat::Reverse,
        dithering: true,
        ..Default::default()
    };
    let mut library = Library::default();
    let id = library.save(None, "Color curve", settings.clone())?;
    let before = library.clone();
    check(
        library.save(None, "color curve", settings.clone()).is_err() && library == before,
        "Duplicate preset changed library",
    )?;
    library.save(
        Some(id),
        "Renamed",
        Settings {
            repeat: Repeat::Repeat,
            ..settings.clone()
        },
    )?;
    let mut history = History::default();
    history.record(before.clone(), &library);
    check(
        history.undo(&mut library) && library == before && history.redo(&mut library),
        "Preset own Undo/Redo",
    )?;
    let path = root.join("gradient-presets.json");
    library.export(&path)?;
    check(
        Library::import(&path)? == library,
        "Preset curves/alpha/repeat reopen",
    )?;
    let saved = std::fs::read(&path).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("json.write.tmp");
    std::fs::write(&temporary, b"in-progress").map_err(|e| e.to_string())?;
    check(
        library.export(&path).is_err()
            && std::fs::read(&path).map_err(|e| e.to_string())? == saved
            && std::fs::read(&temporary).map_err(|e| e.to_string())? == b"in-progress",
        "Preset failed export damaged previous file/temp",
    )?;
    std::fs::remove_file(&temporary).map_err(|e| e.to_string())?;
    let bad = root.join("gradient-presets-bad.json");
    std::fs::write(&bad, b"{\"version\":99}").map_err(|e| e.to_string())?;
    check(Library::import(&bad).is_err(), "Malformed preset accepted")?;
    library.remove(id)?;
    check(library.presets.is_empty(), "Preset removal")?;
    let mut d = crate::document::Document::create(
        crate::document::CanvasSpec {
            width: 32,
            height: 8,
            dpi: 300.0,
            background: crate::document::Background::Transparent,
        },
        crate::document::DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    let before = d.snapshot();
    let s = g::prepare(&d, 0, settings.clone(), true, Default::default())?;
    d.commit_state(s)?;
    let id = d.scene.gradients.layers[0].id;
    let mut changed = settings.clone();
    changed.ramp.as_mut().unwrap().space = Space::PerceptualB;
    changed.ramp.as_mut().unwrap().stops[0].color = [10, 200, 40, 128];
    d.commit_state(g::edit(&d, id, changed)?)?;
    let path = root.join("gradient-ramp.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, crate::document::DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot(),
        "Edited ramp native reopen",
    )?;
    check(
        d.undo()
            && d.scene.gradients.layers[0].settings == settings
            && d.undo()
            && d.snapshot() == before,
        "Ramp document Undo",
    )?;
    // Quantization is applied after interpolation, so dithering can still affect multi-stop ramps.
    let mut s = Settings {
        ramp: Some(Ramp::two([0, 0, 0, 255], [3, 3, 3, 255])),
        start: [0.5, 0.5],
        end: [20.5, 0.5],
        ..Default::default()
    };
    let mut difference = false;
    for x in 0..20 {
        s.dithering = false;
        let a = s.color([x as f64 + 0.5, 0.5], None);
        s.dithering = true;
        let b = s.color([x as f64 + 0.5, 0.5], None);
        difference |= a != b;
        check(a[3] == b[3], "Ramp dither alpha")?;
    }
    check(
        difference,
        "Multi-stop dithering lost by premature quantization",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn independent_spaces_brightness_stops_curves_local_files_history() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
