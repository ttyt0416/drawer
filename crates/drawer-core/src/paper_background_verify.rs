use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    paper_background as p,
    render::Renderer,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = Document::create(
        CanvasSpec {
            width: 4,
            height: 4,
            dpi: 300.,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())?;
    d.layers[0].pixels[5] = [200, 100, 50, 128];
    d.layers[0].pixels[0] = [8, 9, 10, 0];
    let original = d.snapshot();
    let counts = d.history_counts();
    let pixels = Renderer::new(&d).output(300., 4096)?.2;
    let state = p::prepare(&d, [20, 40, 60], true)?;
    check(
        d.snapshot() == original && d.history_counts() == counts,
        "Paper preview mutated original",
    )?;
    d.commit_state(state)?;
    let paper = d.snapshot();
    let id = d.scene.paper_background.ok_or("Missing paper identity")?;
    let n = crate::layers::index(&d, id).unwrap();
    check(
        d.layers[0] == original.layers[0]
            && crate::layers::order(&d)[0] == n
            && Renderer::new(&d).pixel([0.5; 2]) == [20, 40, 60, 255]
            && Renderer::new(&d).pixel([1.5; 2]) == [110, 70, 55, 255],
        "Paper is not below existing semitransparent ink",
    )?;
    let file = root.join("paper-background.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == paper,
        "Paper identity/color native reopen",
    )?;
    let mut bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    bytes[8..12].copy_from_slice(&60u32.to_le_bytes());
    let old = root.join("paper-old-version.drawer");
    std::fs::write(&old, bytes).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&old, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old header accepted paper identity",
    )?;
    check(
        d.undo() && d.snapshot() == original && d.redo() && d.snapshot() == paper,
        "Paper creation Undo/Redo",
    )?;
    let state = p::prepare(&d, [20, 40, 60], false)?;
    d.commit_state(state)?;
    check(
        Renderer::new(&d).output(300., 4096)?.2 == pixels && d.layers[0] == original.layers[0],
        "Hiding paper lost original alpha/hidden RGB",
    )?;
    let hidden = d.snapshot();
    let png = root.join("paper-hidden.png");
    crate::png_export::export(&d, &png, 4096).map_err(|e| e.to_string())?;
    let raw = std::fs::read(&png).map_err(|e| e.to_string())?;
    let image = image::ImageReader::open(&png)
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .into_rgba8();
    check(
        image.get_pixel(0, 0).0 == [0; 4] && image.get_pixel(1, 1).0 == [200, 100, 50, 128],
        "Transparent PNG contains output background while paper hidden",
    )?;
    let settings = crate::workspaces::Snapshot {
        transparency_colors: [[2, 190, 80], [200, 20, 240]],
        ..Default::default()
    };
    let pref = root.join("paper-display-preferences.json");
    settings.export(&pref).map_err(|e| e.to_string())?;
    let loaded = crate::workspaces::Snapshot::read(&pref).map_err(|e| e.to_string())?;
    let (merged, _) = crate::workspaces::Snapshot::default()
        .merge(
            &loaded,
            crate::workspaces::Scope {
                layout: false,
                bars: false,
                keys: false,
                other: true,
            },
            &[],
        )
        .map_err(|e| e.to_string())?;
    check(
        merged.transparency_colors == settings.transparency_colors
            && p::checker_default() == crate::workspaces::Snapshot::default().transparency_colors,
        "Local checker save/import/reset",
    )?;
    crate::png_export::export(&d, &png, 4096).map_err(|e| e.to_string())?;
    check(
        std::fs::read(png).map_err(|e| e.to_string())? == raw && d.snapshot() == hidden,
        "Checker colors leaked into output or document",
    )?;
    check(
        d.undo() && d.snapshot() == paper,
        "Paper visibility one Undo",
    )?;
    let recolored = p::prepare(&d, [240, 220, 180], true)?;
    d.commit_state(recolored)?;
    check(
        d.scene.paper_background == Some(id)
            && d.layers.len() == 2
            && d.layers[n].pixels[0] == [240, 220, 180, 255]
            && d.layers[0] == original.layers[0],
        "Paper recolor duplicated layer or touched ink",
    )?;
    check(d.undo() && d.snapshot() == paper, "Paper recolor Undo")?;
    d.scene
        .layer_properties
        .rasters
        .entry(id)
        .or_default()
        .pixels_locked = true;
    let locked = d.snapshot();
    check(
        p::prepare(&d, [240, 220, 180], true).is_err() && d.snapshot() == locked,
        "Paper recolor bypassed pixel lock",
    )?;
    d.scene.layer_properties.rasters.remove(&id);
    let deleted = crate::layers::prepare(
        &d,
        crate::layers::Command::Delete(crate::layers::Target::Raster(id)),
    )?;
    d.commit_state(deleted.state)?;
    check(
        d.scene.paper_background.is_none() && d.layers[0] == original.layers[0],
        "Paper deletion left stale identity",
    )?;
    check(
        d.undo() && d.snapshot() == paper,
        "Deleted paper Undo identity",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn output_paper_layer_checker_exclusion_png_native_and_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
