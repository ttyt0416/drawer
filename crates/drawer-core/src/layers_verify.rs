use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    layers::{self as core, Command, Kind, Target},
    paper::Node,
};
fn check(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn make() -> Result<Document, String> {
    Document::create(
        CanvasSpec {
            width: 16,
            height: 16,
            dpi: 300.0,
            background: Background::Transparent,
        },
        DEFAULT_DOCUMENT_BUDGET,
    )
    .map_err(|e| e.to_string())
}
fn apply(d: &mut Document, c: Command) -> Result<Target, String> {
    let p = core::prepare(d, c)?;
    let selected = p.selected;
    d.commit_state(p.state)?;
    Ok(selected)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = make()?;
    let first = Target::Raster(1);
    let old = d.snapshot();
    check(
        core::prepare(&d, Command::Delete(first)).is_err() && d.snapshot() == old,
        "Last raster deletion changed page",
    )?;
    for name in ["", "  ", "bad\nname"] {
        check(
            core::prepare(
                &d,
                Command::Create {
                    kind: Kind::Raster,
                    name: name.into(),
                    parent: None,
                },
            )
            .is_err()
                && d.snapshot() == old,
            "Invalid layer name changed page",
        )?;
    }
    let initial_name = d.layers[0].name.clone();
    let second = apply(
        &mut d,
        Command::Create {
            kind: Kind::Raster,
            name: initial_name,
            parent: None,
        },
    )?;
    check(
        second == Target::Raster(2)
            && d.layers.len() == 2
            && d.layers[0].name == d.layers[1].name
            && d.scene.layer_ids.rasters == vec![1, 2],
        "Creation/duplicate names/stable IDs",
    )?;
    let source = d.snapshot();
    let copy = apply(&mut d, Command::Duplicate(second))?;
    check(
        copy == Target::Raster(3) && d.layers[1].pixels == d.layers[2].pixels,
        "Raster duplicate pixels/ID",
    )?;
    d.edit_pixel(2, 0, 0, [200, 40, 20, 255]);
    check(
        d.layers[1].pixel(16, 16, 0, 0)[3] == 0
            && d.layers[2].pixel(16, 16, 0, 0) == [200, 40, 20, 255],
        "Duplicate raster storage alias",
    )?;
    d.undo();
    d.undo();
    check(d.snapshot() == source, "Duplicate/pixel independent Undo")?;
    let folder = d.scene.paper.group_all(d.layers.len(), "Cut A")?;
    let child = apply(
        &mut d,
        Command::Create {
            kind: Kind::Raster,
            name: "Inside".into(),
            parent: Some(folder),
        },
    )?;
    let Target::Raster(child_id) = child else {
        return Err("Wrong raster kind".into());
    };
    let child_index = core::index(&d, child_id).unwrap();
    check(
        core::parent(&d.scene.paper, child_index) == Some(folder),
        "Create inside folder",
    )?;
    apply(
        &mut d,
        Command::Move {
            target: child,
            parent: None,
            position: 0,
        },
    )?;
    check(
        d.scene.paper.roots[0] == Node::Raster(child_index)
            && d.scene.layer_ids.id(child_index) == child_id,
        "Move out of folder changed identity/order",
    )?;
    d.undo();
    check(
        core::parent(&d.scene.paper, child_index) == Some(folder),
        "Folder move Undo",
    )?;
    let mut scene = d.scene.clone();
    scene.lines.settings.references = vec![
        crate::line_tools::Target::Raster(1),
        crate::line_tools::Target::Raster(2),
    ];
    scene.lines.settings.erase = vec![crate::line_tools::Target::Raster(2)];
    for n in [1, 2] {
        let id = scene.rulers.add(
            &format!("Owner {n}"),
            crate::rulers::Kind::Straight {
                a: [0.0, 0.0],
                b: [8.0, 0.0],
            },
        )?;
        let g = scene.rulers.guides.iter_mut().find(|g| g.id == id).unwrap();
        g.link = Some(crate::line_tools::Target::Raster(n));
        g.scope = crate::ruler_scope::Scope::Layer(crate::line_tools::Target::Raster(n));
    }
    scene.alignment.selected = vec![crate::alignment::Item::Raster(2)];
    scene.alignment.reference = crate::alignment::Reference::Key(crate::alignment::Item::Raster(1));
    d.edit_scene(scene)?;
    let before = d.snapshot();
    let history = d.history_counts().0;
    apply(&mut d, Command::Delete(second))?;
    check(
        d.scene.layer_ids.rasters == vec![1, child_id]
            && core::index(&d, child_id) == Some(1)
            && d.scene.lines.settings.references == vec![crate::line_tools::Target::Raster(1)]
            && d.scene.lines.settings.erase == vec![crate::line_tools::Target::Raster(1)]
            && d.scene.rulers.guides.len() == 1
            && d.scene.rulers.guides[0].link == Some(crate::line_tools::Target::Raster(1))
            && d.scene.alignment.reference == crate::alignment::Reference::Selection
            && d.scene.alignment.selected == vec![crate::alignment::Item::Raster(1)],
        "Deletion remapped content references to the wrong owner",
    )?;
    let file = root.join("layers-raster.drawer");
    crate::files::save(&mut d, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == d.snapshot()
            && d.history_counts().0 == history + 1
            && d.undo()
            && d.snapshot() == before,
        "Layer deletion native/group Undo",
    )?;
    let existing = d.scene.layer_ids.rasters.clone();
    let image =
        crate::references::LocalImage::from_rgba(1, 1, &[1, 2, 3, 255], DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?;
    let p = crate::paste::prepare(&d, &image, [0, 0], DEFAULT_DOCUMENT_BUDGET)?;
    d.commit_state(p)?;
    check(
        d.scene.layer_ids.rasters.starts_with(&existing)
            && d.scene.layer_ids.rasters.len() == d.layers.len(),
        "Image paste did not extend stable identities",
    )?;
    d.undo();
    let ids = d.scene.layer_ids.rasters.clone();
    crate::paper::bake(&mut d, Node::Folder(folder))?;
    check(
        d.scene.layer_ids.rasters.len() == d.layers.len()
            && d.scene.layer_ids.rasters.iter().all(|id| !ids.contains(id))
            && d.undo()
            && d.snapshot() == before,
        "Folder bake identity/remapping/Undo",
    )?;

    let mut vector = make()?;
    let v = apply(
        &mut vector,
        Command::Create {
            kind: Kind::Vector,
            name: "Vector".into(),
            parent: None,
        },
    )?;
    let Target::Vector(id) = v else {
        return Err("Wrong vector type".into());
    };
    vector.scene.lines.add_path(
        id,
        vec![[1.0, 1.0], [8.0, 8.0]],
        false,
        1.0,
        [20, 40, 200, 255],
    )?;
    let original = vector.scene.lines.layers[0].paths[0].clone();
    let copy = apply(&mut vector, Command::Duplicate(v))?;
    let Target::Vector(cid) = copy else {
        return Err("Wrong vector copy type".into());
    };
    check(
        cid != id
            && vector.scene.lines.layers[1].paths[0].id != original.id
            && vector.scene.lines.layers[1].paths[0].points == original.points,
        "Vector duplicate identities/geometry",
    )?;
    apply(
        &mut vector,
        Command::Rename {
            target: copy,
            name: "Renamed".into(),
        },
    )?;
    apply(
        &mut vector,
        Command::Move {
            target: copy,
            parent: None,
            position: 0,
        },
    )?;
    check(
        vector.scene.lines.layers[0].id == cid
            && vector.scene.lines.layers[0].name == "Renamed"
            && vector.scene.lines.layers[1].id == id,
        "Vector stack reorder",
    )?;
    vector.scene.lines.layers[0].paths[0].points[0] = [3.0, 4.0];
    check(
        vector.scene.lines.layers[1].paths[0] == original,
        "Vector duplicate storage alias",
    )?;
    let before = vector.snapshot();
    apply(&mut vector, Command::Delete(copy))?;
    check(
        vector.scene.lines.layers.len() == 1
            && vector.scene.lines.layers[0].id == id
            && vector.undo()
            && vector.snapshot() == before,
        "Vector deletion/Undo",
    )?;
    let file = root.join("layers-vector.drawer");
    crate::files::save(&mut vector, &file).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&file, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == before,
        "Vector layer native reopen",
    )?;
    vector.scene.lines.layers[0].locked = true;
    check(
        core::prepare(&vector, Command::Delete(copy)).is_err(),
        "Locked vector deletion",
    )?;
    let mut order = make()?;
    order.layers[0].pixels[0] = [255, 0, 0, 255];
    let second = apply(
        &mut order,
        Command::Create {
            kind: Kind::Raster,
            name: "Blue".into(),
            parent: None,
        },
    )?;
    let Target::Raster(id) = second else {
        unreachable!()
    };
    let i = core::index(&order, id).unwrap();
    order.layers[i].pixels[0] = [0, 0, 255, 255];
    check(
        order.composite_pixel(0, 0) == [0, 0, 255, 255],
        "Initial layer composition order",
    )?;
    apply(
        &mut order,
        Command::Move {
            target: Target::Raster(1),
            parent: None,
            position: 1,
        },
    )?;
    check(
        order.composite_pixel(0, 0) == [255, 0, 0, 255],
        "Reordered layer composition pixels",
    )?;
    check(
        crate::layer_pick::candidates(&order, [0.5, 0.5], Default::default()).first()
            == Some(&crate::thumbnail::Target::Raster(0)),
        "Canvas pick did not follow reordered composition",
    )?;
    let id = order.scene.lines.add_layer("Visible fill")?;
    let state = crate::shapes::prepare(
        &order,
        crate::shapes::Target::Vector(Some(id)),
        &[[1.0, 1.0], [6.0, 6.0]],
        crate::shapes::Settings {
            paint: crate::shapes::Paint::CreateFill,
            line: [0; 4],
            fill: [20, 40, 200, 255],
            ..Default::default()
        },
    )?;
    order.commit_state(state)?;
    check(
        crate::layer_pick::candidates(&order, [3.0, 3.0], Default::default()).first()
            == Some(&crate::thumbnail::Target::Vector(id)),
        "Visible fill with transparent stroke was not selectable",
    )?;
    let unchanged = core::prepare(
        &order,
        Command::Rename {
            target: second,
            name: "Blue".into(),
        },
    )?;
    check(
        unchanged.state == order.snapshot(),
        "Same-name command changed metadata/history",
    )?;
    let mut max = make()?;
    max.layers = vec![max.layers[0].clone(); 256];
    check(
        core::prepare(
            &max,
            Command::Create {
                kind: Kind::Raster,
                name: "Overflow".into(),
                parent: None,
            },
        )
        .is_err()
            && core::prepare(&max, Command::Duplicate(first)).is_err(),
        "Layer maximum",
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn stable_ids_folder_order_duplicate_delete_and_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
