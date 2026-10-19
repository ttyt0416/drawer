use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layer_blend::Mode,
    layer_properties::{self as p, Patch, Target},
    paper::{Folder, Node},
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn reference(mode: Mode, b: [u8; 4], s: [u8; 4]) -> [u8; 4] {
    let ab = b[3] as u64;
    let a = s[3] as u64;
    let alpha = a * 255 + ab * (255 - a);
    if alpha == 0 {
        return [0; 4];
    }
    let mut out = [0; 4];
    for i in 0..3 {
        let cb = b[i] as u64;
        let cs = s[i] as u64;
        let blended = match mode {
            Mode::Normal => cs * 255,
            Mode::Multiply => cb * cs,
            Mode::Screen => (cb + cs) * 255 - cb * cs,
            Mode::Overlay => {
                if cb < 128 {
                    2 * cb * cs
                } else {
                    255 * 255 - 2 * (255 - cb) * (255 - cs)
                }
            }
        };
        let n = cb * ab * (255 - a) * 255 + cs * a * (255 - ab) * 255 + blended * a * ab;
        let den = alpha * 255;
        out[i] = ((n + den / 2) / den) as u8;
    }
    out[3] = ((alpha + 127) / 255) as u8;
    out
}
fn make() -> Result<Document, String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 8,
            height: 8,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels.fill([64, 128, 192, 255]);
    let mut l = d.layers[0].clone();
    l.pixels.fill([192, 64, 128, 255]);
    d.layers.push(l);
    Ok(d)
}
fn set(d: &mut Document, t: Target, m: Mode) -> Result<(), String> {
    let s = p::prepare_patch(
        d,
        &[t],
        Patch {
            blend: Some(m),
            ..Default::default()
        },
    )?;
    d.commit_state(s)?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    for (mode, rgb) in [
        (Mode::Normal, [192, 64, 128]),
        (Mode::Multiply, [48, 32, 96]),
        (Mode::Screen, [208, 160, 224]),
        (Mode::Overlay, [96, 65, 192]),
    ] {
        let mut d = make()?;
        let original = d.snapshot();
        set(&mut d, Target::Raster(2), mode)?;
        let expected = [rgb[0], rgb[1], rgb[2], 255];
        check(
            d.composite_pixel(0, 0) == expected
                && crate::render::Renderer::new(&d).pixel([0.5, 0.5]) == expected,
            "Independent opaque blend colors",
        )?;
        if mode != Mode::Normal {
            check(
                d.undo() && d.snapshot() == original && d.redo(),
                "Blend Undo/Redo",
            )?;
        }
        let file = root.join(format!("blend-{mode:?}.drawer"));
        crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
        check(
            crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
                .map_err(|e| e.to_string())?
                .snapshot()
                == d.snapshot(),
            "Blend identifiers native reopen",
        )?;
        for ab in [0, 64, 128, 255] {
            for a in [0, 64, 128, 255] {
                d.layers[0].pixels.fill([64, 128, 192, ab]);
                d.layers[1].pixels.fill([192, 64, 128, a]);
                let expected = reference(mode, [64, 128, 192, ab], [192, 64, 128, a]);
                let actual = d.composite_pixel(0, 0);
                check(
                    actual.iter().zip(expected).all(|(x, y)| x.abs_diff(y) <= 1),
                    "Independent rational partial-alpha blend table",
                )?;
                check(
                    crate::render::Renderer::new(&d).pixel([0.5, 0.5]) == actual,
                    "Canvas/composite blend disagreement",
                )?;
                check(
                    mode.compose([64, 128, 192, ab], [192, 64, 128, a], 0.)
                        == if ab == 0 { [0; 4] } else { [64, 128, 192, ab] },
                    "Zero layer opacity",
                )?;
            }
        }
        for b in [0, 128, 255] {
            for s in [0, 128, 255] {
                let expected = reference(mode, [b, b, b, 255], [s, s, s, 255]);
                check(
                    mode.compose([b, b, b, 255], [s, s, s, 255], 1.) == expected,
                    "Black/white/midpoint blend table",
                )?;
            }
        }
    }
    let mut folder = make()?;
    folder.scene.paper.folders.push(Folder {
        name: "Blend folder".into(),
        children: vec![Node::Raster(1)],
        through: true,
        paper: Default::default(),
    });
    folder.scene.paper.roots = vec![Node::Raster(0), Node::Folder(0)];
    set(&mut folder, Target::Folder(0), Mode::Multiply)?;
    check(
        !folder.scene.paper.folders[0].through && folder.composite_pixel(0, 0) == [48, 32, 96, 255],
        "Folder mode isolates through group",
    )?;
    let s = p::prepare_patch(
        &folder,
        &[Target::Folder(0)],
        Patch {
            opacity: Some(50),
            ..Default::default()
        },
    )?;
    folder.commit_state(s)?;
    check(
        folder.composite_pixel(0, 0) == [56, 80, 144, 255],
        "Folder opacity blend once",
    )?;
    let mut clipped = make()?;
    clipped.layers[0].pixels.fill([64, 128, 192, 128]);
    let s = crate::clipping::prepare(&clipped, &[Target::Raster(2)], None, Some(true))?;
    clipped.commit_state(s)?;
    set(&mut clipped, Target::Raster(2), Mode::Multiply)?;
    check(
        clipped.composite_pixel(0, 0) == [48, 32, 96, 128],
        "Clipping blend preserves fixed base alpha",
    )?;
    let mut vectors = make()?;
    vectors.layers.pop();
    let a = vectors.scene.lines.add_layer("Vector blend")?;
    vectors.scene.lines.add_path(
        a,
        vec![[0., 0.5], [7., 0.5]],
        false,
        1.,
        [192, 64, 128, 255],
    )?;
    set(&mut vectors, Target::Vector(a), Mode::Multiply)?;
    check(
        crate::render::Renderer::new(&vectors).pixel([0.5, 0.5]) == [48, 32, 96, 255],
        "Vector mode uses backdrop and whole-layer alpha",
    )?;
    let props = serde_json::to_value(p::Properties::default()).map_err(|e| e.to_string())?;
    check(
        props.get("blend").is_none()
            && serde_json::from_value::<p::Properties>(props)
                .map_err(|e| e.to_string())?
                .blend
                == Mode::Normal,
        "Legacy blend default",
    )?;
    check(
        serde_json::from_str::<Mode>("\"UnknownBlend\"").is_err(),
        "Unknown mode silently accepted",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn independent_blend_alpha_folder_vector_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
