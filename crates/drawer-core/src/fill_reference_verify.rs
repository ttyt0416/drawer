use crate::{
    bucket::{self, Expansion, Mode, Reference, Settings},
    document::{Background, CanvasSpec, DEFAULT_DOCUMENT_BUDGET, Document},
    fill_reference::{self as refs, Filter, Role, Tags, Target},
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
pub fn fixture() -> Result<Document, String> {
    let mut d = make(7, 7)?;
    for name in ["Text wall", "Balloon wall", "Draft wall", "Paper wall"] {
        let mut layer = d.layers[0].clone();
        layer.name = name.into();
        d.layers.push(layer);
    }
    for x in 1..6 {
        d.layers[1].pixels[7 + x] = [0, 0, 0, 255];
        d.layers[3].pixels[5 * 7 + x] = [0, 0, 0, 255];
    }
    for y in 1..6 {
        d.layers[2].pixels[y * 7 + 5] = [0, 0, 0, 255];
        d.layers[4].pixels[y * 7 + 1] = [0, 0, 0, 255];
    }
    for (i, role, draft) in [
        (1, Role::Text, false),
        (2, Role::Balloon, false),
        (3, Role::Drawing, true),
        (4, Role::Paper, false),
    ] {
        d.scene.paper.layers.entry(i).or_default().fill = Tags {
            role,
            draft,
            reference: true,
        };
    }
    d.layers[4].locked = true;
    d.scene.paper.folders.push(crate::paper::Folder {
        name: "Walls".into(),
        through: true,
        children: (1..5).map(crate::paper::Node::Raster).collect(),
        paper: Default::default(),
    });
    d.scene.paper.roots = vec![crate::paper::Node::Raster(0), crate::paper::Node::Folder(0)];
    Ok(d)
}
fn region(d: &Document, s: Settings) -> Result<bucket::Region, String> {
    bucket::Region::build(d, 0, [3.5, 3.5], s)
}
pub fn verify(root: &std::path::Path) -> Result<(), String> {
    let mut d = fixture()?;
    let before = d.snapshot();
    let mut s = Settings {
        reference: Reference::Composite,
        antialias: false,
        ..Default::default()
    };
    for n in 1..5 {
        refs::select(&mut s.selected, n, true);
    }
    for reference in [
        Reference::Composite,
        Reference::Marked,
        Reference::Selected,
        Reference::Folder(0),
    ] {
        let r = region(&d, Settings { reference, ..s })?;
        if r.coverage(0, 0) != 0 || r.coverage(3, 3) != 255 {
            return Err(format!(
                "Reference set {reference:?} did not close split wall"
            ));
        }
    }
    for filter in [
        Filter {
            text: true,
            ..Default::default()
        },
        Filter {
            balloon: true,
            ..Default::default()
        },
        Filter {
            draft: true,
            ..Default::default()
        },
        Filter {
            paper: true,
            ..Default::default()
        },
        Filter {
            locked: true,
            ..Default::default()
        },
    ] {
        if region(
            &d,
            Settings {
                exclude: filter,
                ..s
            },
        )?
        .coverage(0, 0)
            != 255
        {
            return Err(format!(
                "Reference exclusion did not remove its wall: {filter:?}"
            ));
        }
    }
    refs::select(&mut s.selected, 4, false);
    if region(
        &d,
        Settings {
            reference: Reference::Selected,
            ..s
        },
    )?
    .coverage(0, 0)
        != 255
    {
        return Err("Selected subset was ignored".into());
    }
    d.scene.paper.layers.get_mut(&4).unwrap().hidden = true;
    for reference in [
        Reference::Composite,
        Reference::Marked,
        Reference::Folder(0),
    ] {
        if region(&d, Settings { reference, ..s })?.coverage(0, 0) != 255 {
            return Err("Hidden reference included".into());
        }
    }
    d.scene.paper.layers.get_mut(&4).unwrap().hidden = false;
    let mut editing = d.fork_page()?;
    editing.layers[0].pixels = editing.layers[1].pixels.clone();
    editing.layers[1].pixels.fill([0; 4]);
    if region(&editing, s)?.coverage(0, 0) != 0
        || region(
            &editing,
            Settings {
                exclude: Filter {
                    editing: true,
                    ..Default::default()
                },
                ..s
            },
        )?
        .coverage(0, 0)
            != 255
    {
        return Err("Editing-layer exclusion".into());
    }
    if d.snapshot() != before {
        return Err("Reference discovery changed page".into());
    }
    for target in 1..5 {
        if d.layers[target].locked
            && bucket::prepare(
                &d,
                target,
                Some([3.5, 3.5]),
                [255, 0, 0, 255],
                false,
                false,
                s,
            )
            .is_ok()
        {
            return Err("Locked fill target changed".into());
        }
    }
    let state = bucket::prepare(&d, 0, Some([3.5, 3.5]), [255, 0, 0, 128], false, false, s)?;
    d.commit_state(state)?;
    let after = d.snapshot();
    if d.layers[0].pixel(7, 7, 3, 3) != [255, 0, 0, 128]
        || d.scene != before.scene
        || !d.undo()
        || d.snapshot() != before
        || !d.redo()
        || d.snapshot() != after
    {
        return Err("Filtered fill pixels/one Undo".into());
    }
    let path = root.join("fill-reference.drawer");
    crate::files::save(&mut d, &path).map_err(|e| e.to_string())?;
    if crate::files::open(&path, DEFAULT_DOCUMENT_BUDGET)
        .map_err(|e| e.to_string())?
        .snapshot()
        != after
    {
        return Err("Fill tags native reopen".into());
    }
    let old = d.scene.clone();
    refs::apply_tags(
        &mut d,
        &[(
            Target::Raster(1),
            Tags {
                reference: false,
                ..Default::default()
            },
        )],
    )?;
    d.undo();
    if d.scene != old {
        return Err("Fill role edit Undo".into());
    }
    let mut dot = make(7, 7)?;
    dot.layers[0].pixels.fill([0, 0, 0, 255]);
    dot.layers[0].pixels[3 * 7 + 3] = [255, 255, 255, 255];
    for (expansion, a, b) in [
        (Expansion::Diamond, 255, 0),
        (Expansion::Rectangle, 255, 255),
        (Expansion::Round, 255, 0),
    ] {
        let r = region(
            &dot,
            Settings {
                expand: 1,
                expansion,
                antialias: false,
                ..Default::default()
            },
        )?;
        if r.coverage(2, 3) != a || r.coverage(2, 2) != b {
            return Err(format!("Expansion corner oracle {expansion:?}"));
        }
    }
    let round = region(
        &dot,
        Settings {
            expand: 2,
            expansion: Expansion::Round,
            antialias: false,
            ..Default::default()
        },
    )?;
    if round.coverage(2, 2) != 255 || round.coverage(1, 1) != 0 || round.coverage(1, 3) != 255 {
        return Err("Round radius two oracle".into());
    }
    let mut gray = make(7, 1)?;
    gray.layers[0].pixels = vec![
        [255, 255, 255, 255],
        [180, 180, 180, 255],
        [50, 50, 50, 255],
        [0, 0, 0, 255],
        [100, 100, 100, 255],
        [200, 200, 200, 255],
        [255, 255, 255, 255],
    ];
    let r = bucket::Region::build(
        &gray,
        0,
        [0.5, 0.5],
        Settings {
            expand: 6,
            expansion: Expansion::Darkest,
            antialias: false,
            ..Default::default()
        },
    )?;
    if r.coverage(3, 0) != 255 || r.coverage(4, 0) != 0 {
        return Err("Darkest expansion crossed a brightness rise".into());
    }
    // A narrow triangular tip is removed by closure, then recovered locally.
    let mut tip = make(9, 9)?;
    tip.layers[0].pixels.fill([0, 0, 0, 255]);
    for y in 2..7 {
        for x in 2..6 {
            tip.layers[0].pixels[y * 9 + x] = [255, 255, 255, 255];
        }
    }
    tip.layers[0].pixels[4 * 9 + 6] = [255, 255, 255, 255];
    let n = Settings {
        mode: Mode::CloseGaps,
        gap: 1,
        antialias: false,
        ..Default::default()
    };
    let seed = [3.5, 4.5];
    let a = bucket::Region::build(&tip, 0, seed, n)?;
    let b = bucket::Region::build(
        &tip,
        0,
        seed,
        Settings {
            fill_narrow: true,
            ..n
        },
    )?;
    if a.coverage(6, 4) != 0 || b.coverage(6, 4) != 255 || b.coverage(7, 4) != 0 {
        return Err("Narrow triangular tip recovery escaped its eligible area".into());
    }
    let mut text = make(16, 16)?;
    text.scene.objects.push(crate::scene::PageObject::Text {
        baseline: [1.0, 12.0],
        text: "A".into(),
        size_px: 12.0,
        color: [0, 0, 0, 255],
    });
    let with = crate::render::Renderer::fill(&text, 0, s)?;
    let without = crate::render::Renderer::fill(
        &text,
        0,
        Settings {
            exclude: Filter {
                text: true,
                ..Default::default()
            },
            ..s
        },
    )?;
    if !(0..16).any(|y| {
        (0..16).any(|x| {
            with.pixel([x as f64 + 0.5, y as f64 + 0.5])[3] > 0
                && without.pixel([x as f64 + 0.5, y as f64 + 0.5])[3] == 0
        })
    }) {
        return Err("Editable text exclusion".into());
    }
    let vector = text
        .scene
        .lines
        .add_layer("Vector")
        .map_err(|e| e.to_string())?;
    text.scene.objects.clear();
    text.scene.lines.add_path(
        vector,
        vec![[1.0, 8.0], [15.0, 8.0]],
        false,
        2.0,
        [0, 0, 0, 255],
    )?;
    refs::apply_tags(
        &mut text,
        &[(
            Target::Vector(vector),
            Tags {
                draft: true,
                reference: true,
                ..Default::default()
            },
        )],
    )?;
    for (reference, excluded) in [(Reference::Marked, false), (Reference::Composite, true)] {
        let r = crate::render::Renderer::fill(
            &text,
            0,
            Settings {
                reference,
                exclude: Filter {
                    draft: excluded,
                    ..Default::default()
                },
                ..s
            },
        )?;
        if (r.pixel([8.5, 8.5])[3] > 0) == excluded {
            return Err("Vector reference/draft filter".into());
        }
    }
    // An excluded ReplaceAlpha recipe must be skipped, rather than rendered as transparent ink.
    let mut recipe = make(1, 1)?;
    recipe.layers[0].pixels[0] = [200, 0, 0, 255];
    recipe.layers.push(recipe.layers[0].clone());
    recipe.scene.paper.layers.entry(1).or_default().blend =
        crate::brush_composite::Mode::ReplaceAlpha;
    recipe.scene.paper.layers.get_mut(&1).unwrap().fill.draft = true;
    let r = crate::render::Renderer::fill(
        &recipe,
        0,
        Settings {
            exclude: Filter {
                draft: true,
                ..Default::default()
            },
            ..s
        },
    )?;
    if r.pixel([0.5; 2]) != [200, 0, 0, 255] {
        return Err("Excluded layer recipe still affected the reference".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn reference_sets_exclusions_expansion_roles_history_native() {
        let root = crate::test_support::TempDir::new();
        super::verify(&root.0).unwrap();
    }
}
