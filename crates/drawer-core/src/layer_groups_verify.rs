use crate::{
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    group_edits as edits,
    layer_groups::{self as g, Command},
    layer_properties::{self as p, Target},
    layer_search::Key,
    scene::Rect,
};
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn fixture() -> Result<Document, String> {
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
    d.layers[0].name = "Linked raster".into();
    d.layers[0].pixels.fill([100, 80, 120, 128]);
    let mut second = d.layers[0].clone();
    second.name = "Outside layer".into();
    second.pixels.fill([0; 4]);
    second.pixels[63] = [20, 230, 40, 255];
    d.layers.push(second);
    let id = d.scene.lines.add_layer("Linked line")?;
    d.scene.lines.add_path(
        id,
        vec![[1.5, 2.5], [6.5, 2.5]],
        false,
        1.,
        [200, 30, 60, 128],
    )?;
    let mut paper = crate::paper::Paper::default();
    paper.pattern.source = 2;
    paper.pattern.scale = 1.3;
    paper.pattern.offset = [0.25, -0.75];
    paper.pattern.angle = 17.;
    paper.pattern.strength = 0.8;
    d.scene.paper.layers.insert(0, paper);
    let mask = crate::layer_folders::rectangle_mask(
        &d,
        Rect {
            min: [0.; 2],
            max: [4., 8.],
        },
        128,
    )?;
    d.scene.layer_effects.rasters.entry(1).or_default().mask = Some(mask);
    let guide = d.scene.rulers.add(
        "Linked guide",
        crate::rulers::Kind::Straight {
            a: [1., 2.],
            b: [6., 2.],
        },
    )?;
    d.scene
        .rulers
        .guides
        .iter_mut()
        .find(|g| g.id == guide)
        .unwrap()
        .link = Some(crate::line_tools::Target::Vector(id));
    Ok(d)
}
pub fn keys(d: &Document) -> Vec<Key> {
    vec![
        Key::Layer(Target::Raster(1)),
        Key::Layer(Target::Vector(d.scene.lines.layers[0].id)),
    ]
}
pub fn save(d: &mut Document, keys: Vec<Key>) -> Result<(), String> {
    d.commit_state(g::prepare(
        d,
        Command::Save {
            id: None,
            name: "Own linked group".into(),
            members: keys,
            linked: [true; 3],
        },
    )?)?;
    Ok(())
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let selected = keys(&d);
    let original = d.snapshot();
    let counts = d.history_counts();
    let preview = g::prepare(
        &d,
        Command::Save {
            id: None,
            name: "Own linked group".into(),
            members: selected.clone(),
            linked: [true; 3],
        },
    )?;
    check(
        d.snapshot() == original,
        "Group creation preview mutated page",
    )?;
    d.commit_state(preview)?;
    let saved = d.snapshot();
    check(
        d.history_counts().0 == counts.0 + 1
            && d.undo()
            && d.snapshot() == original
            && d.redo()
            && d.snapshot() == saved,
        "Group creation single Undo/Redo",
    )?;
    let outside = d.layers[1].clone();
    let old_line = d.scene.lines.layers[0].paths[0].points.clone();
    d.commit_state(crate::search_move::prepare(&d, selected[0], [2, 1])?)?;
    check(
        d.layers[0].extent(8, 8).offset == [2, 1]
            && d.scene.lines.layers[0].paths[0].points[0]
                == [old_line[0][0] + 2., old_line[0][1] + 1.]
            && d.layers[1] == outside,
        "Linked mouse/source move missed member or changed outsider",
    )?;
    check(d.undo() && d.snapshot() == saved, "Linked movement Undo")?;
    let count = d.history_counts().0;
    let mut nudge = crate::precision::Nudge::begin(&d, 0, crate::precision::Target::Layer)?;
    nudge.step(&mut d, [1, 0])?;
    nudge.step(&mut d, [1, 0])?;
    check(
        d.layers[0].extent(8, 8).offset == [2, 0]
            && d.scene.lines.layers[0].paths[0].points[0] == [old_line[0][0] + 2., old_line[0][1]]
            && d.history_counts().0 == count + 1,
        "Frozen linked keyboard movement compounded or split Undo",
    )?;
    check(d.undo() && d.snapshot() == saved, "Keyboard group Undo")?;
    let matrix = edits::matrix([4., 4.], 90., 1., false)?;
    let before = crate::layer_merge::preview(&d.snapshot(), 0)?;
    let prepared = edits::transform(&d, &[selected[0]], matrix)?;
    check(
        d.snapshot() == saved,
        "Group transform preview mutated source",
    )?;
    d.commit_state(prepared)?;
    check(
        d.layers[1] == outside
            && d.scene.lines.layers[0].paths[0].points[0] == matrix.map(old_line[0]),
        "Quarter turn changed outsider or lost editable line",
    )?;
    let a = crate::render::Renderer::new(&before);
    let b = crate::render::Renderer::new(&d);
    for y in 0..8 {
        for x in 0..8 {
            let old = [x as i64, y as i64];
            let q = matrix.map([x as f64 + 0.5, y as f64 + 0.5]);
            let new = q.map(|v| v.floor() as i64);
            let want = before.scene.paper.intrinsic_node_pixel_with(
                &before,
                crate::paper::Node::Raster(0),
                old,
                &mut |n, _| a.raster_pixel(n, old),
            );
            let actual = d.scene.paper.intrinsic_node_pixel_with(
                &d,
                crate::paper::Node::Raster(0),
                new,
                &mut |n, _| b.raster_pixel(n, new),
            );
            check(
                want == actual,
                "Texture/mask frame did not follow quarter turn",
            )?;
            check(
                a.source_pixel_unbounded(selected[1], [x as f64 + 0.5, y as f64 + 0.5])
                    == b.source_pixel_unbounded(selected[1], q),
                "Rotated vector ink alpha/geometry changed",
            )?;
        }
    }
    drop(a);
    drop(b);
    let old_ruler = crate::ruler_scope::transform(&before, &before.scene.rulers.guides[0])
        .unwrap()
        .map([1., 2.]);
    let new_ruler = crate::ruler_scope::transform(&d, &d.scene.rulers.guides[0])
        .unwrap()
        .map([1., 2.]);
    check(
        new_ruler == matrix.map(old_ruler),
        "Linked ruler did not rotate",
    )?;
    let transformed = d.snapshot();
    let path = root.join("linked-groups.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
            .map_err(|e| e.to_string())?
            .snapshot()
            == transformed,
        "Group/frame native roundtrip",
    )?;
    let intact = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut legacy = intact.clone();
    legacy[8..12].copy_from_slice(&58u32.to_le_bytes());
    let legacy_path = root.join("linked-groups-old-version.drawer");
    std::fs::write(&legacy_path, legacy).map_err(|e| e.to_string())?;
    check(
        crate::files::open(&legacy_path, DEFAULT_DOCUMENT_BUDGET).is_err(),
        "Old version accepted group/local-frame data",
    )?;
    let mut bad = crate::layer_merge::preview(&transformed, 0)?;
    bad.scene.layer_groups.transforms[0].key = Key::Layer(Target::Raster(999999));
    check(
        crate::files::save(&mut bad, &path).is_err()
            && std::fs::read(&path).map_err(|e| e.to_string())? == intact,
        "Invalid live frame damaged native file",
    )?;
    check(d.undo() && d.snapshot() == saved, "Transform single Undo")?;
    let filter = crate::layer_effects::Adjustment {
        brightness: 0.1,
        ..Default::default()
    };
    d.commit_state(edits::filter(&d, &[selected[1]], filter)?)?;
    check(
        d.layers[0].pixels[0] == [126, 106, 146, 128]
            && d.scene.lines.layers[0].paths[0].color == [226, 56, 86, 128]
            && d.layers[1] == outside,
        "Linked ink filter did not retain alpha/outsider",
    )?;
    check(d.undo() && d.snapshot() == saved, "Linked filter Undo")?;
    let v = c_target(&d, selected[1]);
    d.commit_state(p::prepare_patch(
        &d,
        &[v],
        p::Patch {
            locked: Some(true),
            ..Default::default()
        },
    )?)?;
    let locked = d.snapshot();
    check(
        g::move_linked(&d, &[selected[0]], [1, 0]).is_err()
            && edits::transform(&d, &[selected[0]], matrix).is_err()
            && edits::filter(&d, &[selected[0]], filter).is_err()
            && crate::precision::Nudge::begin(&d, 0, crate::precision::Target::Layer).is_err()
            && d.snapshot() == locked,
        "Locked linked member allowed partial operation",
    )?;
    let mut d = fixture()?;
    let mut child = d.layers[1].clone();
    child.name = "Hidden child".into();
    child.pixels[4] = [80, 50, 120, 255];
    d.layers.push(child);
    d.commit_state(
        crate::layers::prepare(
            &d,
            crate::layers::Command::Move {
                target: crate::layers::Target::Raster(2),
                parent: None,
                position: 2,
            },
        )?
        .state,
    )?;
    let f = crate::layer_folders::prepare(
        &d,
        crate::layer_folders::Command::Wrap {
            name: "Group folder".into(),
            targets: vec![Target::Raster(1), Target::Raster(3)],
        },
    )?;
    let folder = f.selected;
    d.commit_state(f.state)?;
    d.commit_state(p::prepare_patch(
        &d,
        &[Target::Raster(3)],
        p::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?)?;
    save(
        &mut d,
        vec![Key::Layer(folder), Key::Layer(Target::Raster(2))],
    )?;
    let original = d.snapshot();
    d.commit_state(g::prepare(&d, Command::Solo(vec![Key::Layer(folder)]))?)?;
    check(
        p::properties(&d, Target::Raster(3)).hidden
            && p::properties(&d, v).hidden
            && !p::properties(&d, folder).hidden,
        "Solo lost hidden folder child or showed outsider",
    )?;
    check(
        crate::clipping::solo(&d, Some(0)).is_err(),
        "Nested solo systems conflicted",
    )?;
    d.commit_state(g::prepare(&d, Command::Restore)?)?;
    check(
        d.snapshot() == original,
        "Solo original states not restored",
    )?;
    d.commit_state(crate::clipping::solo(&d, Some(0))?)?;
    check(
        p::properties(&d, Target::Raster(3)).hidden,
        "Folder solo revealed hidden child",
    )?;
    d.commit_state(crate::clipping::solo(&d, None)?)?;
    check(
        d.snapshot() == original,
        "Folder solo restore changed group state",
    )?;
    d.commit_state(g::move_linked(&d, &[Key::Layer(folder)], [1, 0])?)?;
    check(
        p::properties(&d, Target::Raster(3)).hidden && d.layers[2].extent(8, 8).offset == [1, 0],
        "Hidden folder child failed to follow movement",
    )?;
    d.undo();
    let transformed = edits::transform(&d, &[Key::Layer(folder)], matrix)?;
    d.commit_state(transformed)?;
    check(
        p::properties(&d, Target::Raster(3)).hidden,
        "Hidden child visibility lost after quarter turn",
    )?;
    let mut malicious = d.snapshot();
    let n = 2;
    malicious.layers[n].pixels[0] = [255, 0, 0, 255];
    check(
        d.commit_state(malicious).is_err(),
        "Geometry provenance allowed a hidden ink edit",
    )?;
    d.undo();
    // Deleting a member retires the linkage; solo restoration skips deleted IDs.
    d.commit_state(g::prepare(&d, Command::Solo(vec![Key::Layer(folder)]))?)?;
    d.commit_state(
        crate::layers::prepare(
            &d,
            crate::layers::Command::Delete(crate::layers::Target::Raster(2)),
        )?
        .state,
    )?;
    check(
        d.scene.layer_groups.groups.is_empty(),
        "Deleted member left a dangling link group",
    )?;
    d.commit_state(g::prepare(&d, Command::Restore)?)?;
    check(
        p::properties(&d, Target::Raster(3)).hidden,
        "Restore after deletion lost hidden child state",
    )?;
    // Alpha protection permits a proven geometry change and still rejects alpha painting.
    let mut protected = fixture()?;
    let members = keys(&protected);
    save(&mut protected, members.clone())?;
    protected.layers[0].pixels[0] = [50, 80, 120, 32];
    protected.layers[0].pixels[1] = [10, 30, 50, 255];
    protected.scene.clipping.alpha.insert(Target::Raster(1));
    protected.commit_state(edits::transform(&protected, &[members[0]], matrix)?)?;
    check(
        protected.scene.clipping.alpha.contains(&Target::Raster(1)),
        "Transform removed alpha protection",
    )?;
    let mut forged = protected.snapshot();
    forged.layers[0].pixels[0][3] ^= 127;
    check(
        protected.commit_state(forged).is_err(),
        "Geometry proof weakened subsequent alpha protection",
    )?;
    // Hidden/deleted clipping bases do not consume the stored upper linework.
    let mut clipped = fixture()?;
    let base = clipped.scene.lines.layers[0].id;
    let top = clipped.scene.lines.add_layer("Surviving clipped ink")?;
    clipped.scene.lines.add_path(
        top,
        vec![[2.5, 1.5], [2.5, 6.5]],
        false,
        1.,
        [30, 80, 200, 128],
    )?;
    clipped
        .scene
        .clipping
        .bind(Target::Vector(top), Some(Target::Vector(base)));
    clipped.commit_state(p::prepare_patch(
        &clipped,
        &[Target::Vector(base)],
        p::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )?)?;
    let line = clipped.scene.lines.layers[1].paths[0].clone();
    clipped.commit_state(
        crate::layers::prepare(
            &clipped,
            crate::layers::Command::Delete(crate::layers::Target::Vector(base)),
        )?
        .state,
    )?;
    save(
        &mut clipped,
        vec![
            Key::Layer(Target::Raster(1)),
            Key::Layer(Target::Vector(top)),
        ],
    )?;
    clipped.commit_state(edits::transform(
        &clipped,
        &[Key::Layer(Target::Raster(1))],
        matrix,
    )?)?;
    let after_line = &clipped.scene.lines.layers[0].paths[0];
    check(
        after_line.id == line.id
            && after_line.color == line.color
            && after_line.points
                == line
                    .points
                    .iter()
                    .copied()
                    .map(|p| matrix.map(p))
                    .collect::<Vec<_>>(),
        "Deleted clipping base lost upper ink during quarter turn",
    )?;
    // Unsupported editable source kinds reject an entire transform/filter before mutation.
    let mut unsupported = crate::layer_move_verify::fixture()?;
    save(
        &mut unsupported,
        vec![Key::Layer(Target::Raster(1)), Key::Object(4)],
    )?;
    check(
        edits::transform(&unsupported, &[Key::Layer(Target::Raster(1))], matrix).is_ok(),
        "Linked editable text transform rejected",
    )?;
    let object =
        crate::layer_catalog::source_index(&unsupported, Key::Object(4)).ok_or("Linked source")?;
    unsupported.scene.objects[object] = crate::scene::PageObject::Tone {
        bounds: Rect {
            min: [0.; 2],
            max: [8.; 2],
        },
        period_px: 4.,
        coverage: 0.5,
    };
    let untouched = unsupported.snapshot();
    check(
        edits::transform(&unsupported, &[Key::Layer(Target::Raster(1))], matrix).is_err()
            && edits::filter(&unsupported, &[Key::Layer(Target::Raster(1))], filter).is_err()
            && unsupported.snapshot() == untouched,
        "Unsupported linked source partially edited the group",
    )?;
    let mut detached = crate::layer_merge::preview(&saved, 0)?;
    detached.scene.layer_groups.groups[0].linked = [false; 3];
    let detached_state = detached.snapshot();
    let moved = g::move_linked(&detached, &[selected[0]], [1, 0])?;
    check(
        moved.scene.lines == detached_state.scene.lines,
        "Disabled movement link moved vector",
    )?;
    let filtered = edits::filter(&detached, &[selected[0]], filter)?;
    check(
        filtered.scene.lines == detached_state.scene.lines,
        "Disabled filter link filtered vector",
    )?;
    let turned = edits::transform(&detached, &[selected[0]], matrix)?;
    check(
        turned.scene.lines == detached_state.scene.lines,
        "Disabled transform link turned vector",
    )?;
    detached.commit_state(g::prepare(
        &detached,
        Command::Visibility {
            members: vec![selected[0]],
            hidden: true,
        },
    )?)?;
    check(
        p::properties(&detached, Target::Raster(1)).hidden && p::properties(&detached, v).hidden,
        "Batch visibility missed group members",
    )?;
    detached.commit_state(g::prepare(
        &detached,
        Command::Visibility {
            members: vec![selected[0]],
            hidden: false,
        },
    )?)?;
    check(
        detached.snapshot() == detached_state,
        "Batch own visibility restore",
    )?;
    let mut invalid = original.scene.layer_groups.clone();
    let mut duplicate = invalid.groups[0].clone();
    duplicate.id += 1;
    invalid.next = duplicate.id;
    invalid.groups.push(duplicate);
    check(!invalid.valid(), "Overlapping groups accepted")?;
    Ok(())
}
fn c_target(d: &Document, k: Key) -> Target {
    crate::layer_catalog::target(d, k).unwrap()
}
#[cfg(test)]
mod tests {
    #[test]
    fn linked_atomic_move_transform_filter_solo_hidden_texture_native_undo() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
