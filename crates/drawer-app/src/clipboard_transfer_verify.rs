use super::*;
use std::path::Path;
pub(super) fn run(root: &Path) -> Result<(), String> {
    drawer_core::clipboard_transfer::verify::run(root)?;
    let ctx = egui::Context::default();
    let mut app = Drawer::new(&ctx, None);
    app.simple_view = true;
    app.add_document(drawer_core::clipboard_transfer::verify::fixture()?);
    let before = app.documents[0].snapshot();
    let identity = (app.documents[0].history_counts(), app.documents[0].revision);
    let mut clip = vec![];
    app.copy_selection_from(|p, b| {
        clip = p.encode(b)?;
        Ok(())
    })?;
    if clip.is_empty() || app.documents[0].snapshot() != before {
        return Err("Named copy/source isolation".into());
    }
    if app
        .cut_raster_from(|_, _| Err("Clipboard locked".into()))
        .is_ok()
        || app.documents[0].snapshot() != before
        || identity != (app.documents[0].history_counts(), app.documents[0].revision)
    {
        return Err("Failed clipboard write cut source/history".into());
    }
    app.cut_raster_from(|p, b| {
        clip = p.encode(b)?;
        Ok(())
    })?;
    let cut = app.documents[0].snapshot();
    let committed_clip = clip.clone();
    if cut == before
        || app.documents[0].history_counts().0 != identity.0.0 + 1
        || !app.documents[0].undo()
        || app.documents[0].snapshot() != before
        || clip != committed_clip
    {
        return Err("Cut/Undo changed clipboard or failed atomic source restoration".into());
    }
    let read = || ClipboardImage::decode(&clip, DEFAULT_DOCUMENT_BUDGET / 4);
    if app
        .begin_paste(&ctx, || Err("Clipboard read locked".into()))
        .is_ok()
        || app.paste_dialog.is_some()
    {
        return Err("Locked clipboard opened a mutable paste".into());
    }
    app.begin_paste(&ctx, read)?;
    if app.paste_dialog.as_ref().unwrap().offset != [3, 2] {
        return Err("Normal internal paste lost original document coordinates".into());
    }
    let mut t = 0.;
    let frame = |a: &mut Drawer, t: &mut f64, e| {
        *t += 0.05;
        input_replay::frame(&ctx, a, *t, e);
    };
    let click = |a: &mut Drawer, t: &mut f64, apply: bool| -> Result<(), String> {
        for _ in 0..3 {
            frame(a, t, vec![]);
        }
        let p = a.paste_dialog.as_ref().ok_or("Paste preview disappeared")?;
        let pos = if apply {
            p.apply_button
        } else {
            p.cancel_button
        }
        .ok_or("Missing paste button")?
        .center();
        for pressed in [true, false] {
            frame(
                a,
                t,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        for _ in 0..3 {
            frame(a, t, vec![]);
        }
        Ok(())
    };
    click(&mut app, &mut t, true)?;
    if app.paste_dialog.is_some()
        || app.documents[0].layers.last().unwrap().name != "선화 🎨"
        || app.documents[0]
            .layers
            .last()
            .unwrap()
            .placement
            .unwrap()
            .offset
            != [3, 2]
    {
        return Err("Pointer paste lost name/origin".into());
    }
    if !app.documents[0].undo() || app.documents[0].snapshot() != before {
        return Err("Pointer paste Undo".into());
    }
    for (ppp, zoom, angle, flipped) in [
        (1., 1., 0., false),
        (1.5, 2., 77., true),
        (2., 0.5, -90., false),
    ] {
        ctx.set_pixels_per_point(ppp);
        frame(&mut app, &mut t, vec![]);
        let center = [-30., 80.];
        let d = &mut app.documents[0];
        d.view.zoom = zoom;
        d.view.angle = angle;
        d.view.flipped = flipped;
        d.view.center_on(
            center,
            [d.width as f64, d.height as f64],
            [0.; 2],
            ppp as f64,
        );
        app.begin_paste_view_from(&ctx, read)?;
        if app.paste_dialog.as_ref().unwrap().offset != [-31, 80] {
            return Err(format!(
                "View-center paste geometry: ppp={ppp}, zoom={zoom}, angle={angle}, flip={flipped}, offset={:?}",
                app.paste_dialog.as_ref().unwrap().offset
            ));
        }
        click(&mut app, &mut t, false)?;
        if app.documents[0].snapshot() != before || app.paste_dialog.is_some() {
            return Err("View paste cancel changed document".into());
        }
    }
    let mut bounds = vec![];
    app.copy_bounds_from(|p, b| {
        bounds = p.encode(b)?;
        Ok(())
    })?;
    let bound = ClipboardImage::decode(&bounds, DEFAULT_DOCUMENT_BUDGET / 4)?;
    if bound.image.width != 6 || bound.image.height != 4 || bound.origin != Some([1, 1]) {
        return Err("Selection-bounds command discarded transparent whitespace".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn named_clipboard_cut_failure_pointer_paste_undo_view_center_cancel() {
        let t = crate::tests::TestDirectory::new();
        super::run(&t.0).unwrap();
    }
}
