use super::*;
pub(super) fn run(root: &std::path::Path) -> Result<(), String> {
    drawer_core::palette_exchange_verify::verify(root)?;
    let dir = root.join("palette-ui");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let source = dir.join("source.json");
        let source_tables = drawer_core::palette_exchange_verify::fixture();
        Bundle::capture(&source_tables, false)?.write(&source, false)?;
        let original_file = std::fs::read(&source).map_err(|e| e.to_string())?;
        let user = dir.join("data");
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
        app.simple_view = true;
        app.add_document(drawer_core::png_io_verify::fixture()?);
        let mut time = 0.;
        let frame = |a: &mut Drawer, t: &mut f64, events| {
            *t += 0.05;
            input_replay::frame(&ctx, a, *t, events);
        };
        let settle = |a: &mut Drawer, t: &mut f64| {
            for _ in 0..3 {
                frame(a, t, vec![]);
            }
        };
        let click = |a: &mut Drawer, t: &mut f64, key: &str| -> Result<(), String> {
            let r = a
                .palette_exchange
                .controls
                .get(key)
                .copied()
                .ok_or_else(|| format!("Missing palette control {key}"))?;
            if r.center().y > 690. {
                return Err(format!("Clipped palette control {key}"));
            }
            for pressed in [true, false] {
                frame(
                    a,
                    t,
                    vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            settle(a, t);
            Ok(())
        };
        settle(&mut app, &mut time);
        let doc = app.documents[0].snapshot();
        let before = app.capture_workspace().color_tables;
        app.begin_palette_exchange();
        app.palette_exchange.path = source.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "read")?;
        if app.palette_exchange.prepared.is_none()
            || app.workspaces.store.current.color_tables != before
        {
            return Err("Palette read changed current settings".into());
        }
        click(&mut app, &mut time, "cancel")?;
        if app.workspaces.store.current.color_tables != before {
            return Err("Canceled palette registered entries".into());
        }
        app.begin_palette_exchange();
        app.palette_exchange.path = source.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "read")?;
        click(&mut app, &mut time, "apply")?;
        let copied = app.workspaces.store.current.color_tables.clone();
        if copied.tables.len() != before.tables.len() + 1
            || copied.tables[1].id == source_tables.tables[0].id
        {
            return Err("Palette copy reused an existing ID".into());
        }
        app.color_tables
            .history
            .undo(&mut app.workspaces.store.current.color_tables);
        if app.workspaces.store.current.color_tables != before {
            return Err("Palette settings Undo not one step".into());
        }
        app.color_tables
            .history
            .redo(&mut app.workspaces.store.current.color_tables);
        app.begin_palette_exchange();
        app.palette_exchange.path = source.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "names")?;
        click(&mut app, &mut time, "read")?;
        click(&mut app, &mut time, "apply")?;
        if app.workspaces.store.current.color_tables != copied || !app.palette_exchange.open {
            return Err("Palette replacement bypassed confirmation".into());
        }
        click(&mut app, &mut time, "confirm")?;
        click(&mut app, &mut time, "apply")?;
        let replaced = app.workspaces.store.current.color_tables.clone();
        if replaced.tables.len() != copied.tables.len()
            || replaced.tables[0].id != before.tables[0].id
            || replaced.tables[0].colors[0].color != [200, 10, 30, 128]
        {
            return Err("Palette confirmed replacement differs".into());
        }
        let ctx2 = egui::Context::default();
        let restarted = Drawer::new(&ctx2, None)
            .with_storage(UserStorage::at(user.clone()).map_err(|e| e.to_string())?);
        if restarted.workspaces.store.current.color_tables != replaced {
            return Err("Palette import did not survive restart".into());
        }
        let exported = dir.join("exported.json");
        app.begin_palette_exchange();
        app.palette_exchange.path = exported.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "export")?;
        if Bundle::read(&exported)?.tables != replaced.tables {
            return Err("Palette UI export differs".into());
        }
        click(&mut app, &mut time, "cancel")?;
        app.begin_palette_exchange();
        app.palette_exchange.path = source.display().to_string();
        settle(&mut app, &mut time);
        click(&mut app, &mut time, "read")?;
        let saved = app.workspaces.store.current.color_tables.clone();
        app.workspaces
            .store
            .current
            .color_tables
            .add([1, 2, 3, 4])?;
        let changed = app.workspaces.store.current.color_tables.clone();
        click(&mut app, &mut time, "apply")?;
        if app.workspaces.store.current.color_tables != changed {
            return Err("Stale palette import clobbered a new edit".into());
        }
        click(&mut app, &mut time, "cancel")?;
        app.workspaces.store.current.color_tables = saved;
        if app.documents[0].snapshot() != doc
            || std::fs::read(&source).map_err(|e| e.to_string())? != original_file
        {
            return Err("Palette exchange changed drawing / original file".into());
        }
        Ok(())
    })();
    result.and(std::fs::remove_dir_all(dir).map_err(|e| e.to_string()))
}
