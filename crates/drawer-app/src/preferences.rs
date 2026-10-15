use super::*;
use drawer_core::preferences::{Confirmation, ImeBackend, ImeMode, ImeSession, Language, Settings};
#[derive(Clone)]
struct Target {
    id: u64,
    number: bool,
    composing: bool,
}
#[derive(Default)]
struct NativeIme {
    #[cfg(windows)]
    context: Option<michiu_window::ImeContext>,
}
impl ImeBackend for NativeIme {
    fn begin(&mut self) -> Result<bool, String> {
        #[cfg(windows)]
        {
            let window =
                winsafe::HWND::GetFocus().ok_or("No focused window on the Drawer UI thread")?;
            let context =
                michiu_window::ImeContext::new(windows::Win32::Foundation::HWND(window.ptr()))
                    .map_err(|e| e.to_string())?;
            let open = context.is_open();
            self.context = Some(context);
            Ok(open)
        }
        #[cfg(not(windows))]
        {
            Err("Windows IME control is unavailable".into())
        }
    }
    fn set(&mut self, open: bool) -> Result<(), String> {
        #[cfg(windows)]
        {
            let context = self
                .context
                .as_ref()
                .ok_or("No acquired Drawer IME context")?;
            let original = context.is_open();
            context.set_open(open);
            if context.is_open() != open {
                context.set_open(original);
                return Err("Windows did not accept the IME state; previous state retained".into());
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = open;
            Err("Windows IME control is unavailable".into())
        }
    }
    fn end(&mut self) {
        #[cfg(windows)]
        {
            self.context = None;
        }
    }
}
#[derive(Default)]
pub(super) struct Preferences {
    pub open: bool,
    draft: Option<Settings>,
    session: ImeSession,
    native: NativeIme,
    seen: Option<(u64, ImeMode)>,
    pub error: Option<String>,
    #[cfg(test)]
    apply: Option<egui::Rect>,
    #[cfg(test)]
    cancel: Option<egui::Rect>,
    #[cfg(test)]
    restore_all: Option<egui::Rect>,
    #[cfg(test)]
    restore_one: Vec<(Confirmation, egui::Rect)>,
}
impl Drop for Preferences {
    fn drop(&mut self) {
        let _ = self.session.finish(&mut self.native);
    }
}
pub(super) fn observe(
    ctx: &egui::Context,
    response: &egui::Response,
    number: bool,
    composing: bool,
) {
    if response.has_focus() && ctx.input(|i| i.focused) {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<Target>>(egui::Id::new("drawer-ime-targets"))
                .push(Target {
                    id: response.id.value(),
                    number,
                    composing,
                })
        });
    }
}
pub(super) fn composing(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new("drawer-ime-composing")))
        .unwrap_or(false)
        || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Ime(_))))
}
impl Drawer {
    pub(super) fn begin_input_preferences(&mut self, ctx: &egui::Context) {
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("drawer-ime-targets"), Vec::<Target>::new());
        });
    }
    pub(super) fn update_input_preferences(&mut self, ctx: &egui::Context) {
        let target = ctx
            .data(|d| d.get_temp::<Vec<Target>>(egui::Id::new("drawer-ime-targets")))
            .and_then(|t| t.last().cloned());
        let composing = target.as_ref().is_some_and(|t| t.composing);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("drawer-ime-composing"), composing));
        let prefs = &self.workspaces.store.current.preferences;
        let next = target.map(|t| {
            (
                t.id,
                if t.number {
                    prefs.number_ime
                } else {
                    prefs.text_ime
                },
            )
        });
        // eframe associates the native IME context after this pass. Acquire on the next pass.
        let result = if self.preferences.seen != next {
            self.preferences.seen = next;
            ctx.request_repaint();
            self.preferences
                .session
                .update(&mut self.preferences.native, None, false)
        } else {
            self.preferences
                .session
                .update(&mut self.preferences.native, next, composing)
        };
        if let Err(e) = result {
            self.status = Some(format!("IME preference unavailable: {e}"));
            self.preferences.error = Some(e);
        }
    }
    pub(super) fn open_tutorial(&mut self) {
        self.open_help(match self.workspaces.store.current.preferences.language {
            Language::Korean => "tutorial-ko",
            Language::English => "tutorial-en",
        });
    }
    pub(super) fn preference_settings(&mut self, ctx: &egui::Context) {
        if !self.preferences.open {
            return;
        }
        let mut draft = self
            .preferences
            .draft
            .take()
            .unwrap_or_else(|| self.workspaces.store.current.preferences.clone());
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        #[cfg(test)]
        self.preferences.restore_one.clear();
        egui::Window::new("Language, IME and confirmations").open(&mut open).constrain(true).show(ctx,|ui|{
            ui.horizontal(|ui|{let a=crate::help::button(ui,"Apply preferences","preferences");let c=crate::help::button(ui,"Cancel preferences","preferences");apply=a.clicked();cancel=c.clicked();#[cfg(test)]{self.preferences.apply=Some(a.rect);self.preferences.cancel=Some(c.rect);}});
            ui.label("Local tutorial / tool description language");ui.horizontal(|ui|{ui.selectable_value(&mut draft.language,Language::Korean,"한국어");ui.selectable_value(&mut draft.language,Language::English,"English");});
            ui.label("IME when entering a text field");ui.horizontal(|ui|{for(mode,label)in[(ImeMode::Keep,"Keep current"),(ImeMode::Open,"Open IME"),(ImeMode::Closed,"Close IME")] {ui.selectable_value(&mut draft.text_ime,mode,label);}});
            ui.label("IME when entering a number field");ui.horizontal(|ui|{ui.selectable_value(&mut draft.number_ime,ImeMode::Keep,"Keep current");ui.selectable_value(&mut draft.number_ime,ImeMode::Closed,"Close IME");});
            ui.label("Each field restores the entry state on exit or window focus loss. Composition is never forced into a number. Failure keeps the previous state and shows a reason.");
            if let Some(error)=&self.preferences.error{ui.colored_label(egui::Color32::LIGHT_RED,error);}
            ui.separator();ui.label("Confirmation dialogs (suppression affects settings drafts only)");
            for warning in Confirmation::ALL {ui.horizontal(|ui|{
                let mut hidden=draft.suppressed.contains(&warning);if crate::help::checkbox(ui,&mut hidden,format!("Hide: {}",warning.label()),"preferences").changed(){if hidden{draft.suppressed.insert(warning);}else{draft.suppressed.remove(&warning);}}
                let restore=crate::help::button(ui,"Restore this confirmation","preferences");#[cfg(test)]self.preferences.restore_one.push((warning,restore.rect));if restore.clicked(){draft.restore(Some(warning));}
            });}
            let restore=crate::help::button(ui,"Restore all confirmations","preferences");#[cfg(test)]{self.preferences.restore_all=Some(restore.rect);}if restore.clicked(){draft.restore(None);}
            if crate::help::button(ui,"Open offline tutorial","preferences").clicked(){self.open_help(match draft.language{Language::Korean=>"tutorial-ko",Language::English=>"tutorial-en"});}
        });
        if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.preferences.open = false;
            self.preferences.draft = None;
        } else if apply {
            let mut snapshot = self.capture_workspace();
            snapshot.preferences = draft.clone();
            self.stage_workspace(snapshot, self.workspaces.store.active.clone());
            match self.apply_workspace(
                drawer_core::workspaces::Scope {
                    other: true,
                    ..Default::default()
                },
                None,
            ) {
                Ok(()) => {
                    self.preferences.open = false;
                    self.preferences.draft = None;
                }
                Err(e) => {
                    self.status = Some(e);
                    self.workspaces.staged = None;
                    self.preferences.draft = Some(draft);
                }
            }
        } else {
            self.preferences.draft = Some(draft);
        }
    }
}
pub(super) fn tool_hint(language: Language, tip: BrushTip) -> &'static str {
    match (language, tip) {
        (Language::Korean, BrushTip::Pen) => "펜: 선명한 가장자리. B는 기본 선택 키입니다.",
        (Language::Korean, BrushTip::Pencil) => "연필: 입자가 있는 팁. N은 기본 선택 키입니다.",
        (Language::Korean, BrushTip::Eraser) => {
            "지우개: 투명하게 지웁니다. E는 기본 선택 키입니다."
        }
        (Language::Korean, BrushTip::Wash) => {
            "에어브러시: 부드럽게 누적되는 반투명 팁. W는 기본 선택 키입니다."
        }
        (Language::Korean, BrushTip::Marker) => "마커: 같은 획에서 다시 지나가면 농도가 쌓입니다.",
        (Language::Korean, BrushTip::Smudge) => {
            "번짐: 현재 층의 색을 운반하며 선택색은 공급하지 않습니다."
        }
        (Language::Korean, BrushTip::PixelPen) => {
            "픽셀 펜: 격자에 맞춘 단단한 경계. 크기1은 한 픽셀입니다."
        }
        (Language::English, BrushTip::Marker) => {
            "Marker: revisiting the same path builds pigment density."
        }
        (Language::English, BrushTip::Smudge) => {
            "Smudge: carries current-layer pigment without foreground supply."
        }
        (Language::English, BrushTip::PixelPen) => {
            "Pixel pen: grid-aligned binary footprint; size1 is one pixel."
        }
        (Language::English, BrushTip::Pen) => "Pen: crisp edge. Default tool key B.",
        (Language::English, BrushTip::Pencil) => "Pencil: grainy tip. Default tool key N.",
        (Language::English, BrushTip::Eraser) => "Eraser: removes opacity. Default tool key E.",
        (Language::English, BrushTip::Wash) => {
            "Airbrush: soft translucent buildup. Default tool key W."
        }
    }
}

/// Smoke-test only the application's own hidden native window; restore before returning.
#[cfg(windows)]
pub(super) fn native_probe(frame: &eframe::Frame) -> Result<&'static str, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let raw = frame.window_handle().map_err(|e| e.to_string())?.as_raw();
    let RawWindowHandle::Win32(handle) = raw else {
        return Err("Drawer window is not Win32".into());
    };
    let context = match michiu_window::ImeContext::new(windows::Win32::Foundation::HWND(
        handle.hwnd.get() as *mut std::ffi::c_void,
    )) {
        Ok(context) => context,
        Err(e) => {
            eprintln!("Hidden Drawer IME provider unavailable: {e}");
            return Ok("provider-declined-state-preserved");
        }
    };
    let original = context.is_open();
    context.set_open(!original);
    let changed = context.is_open() != original;
    context.set_open(original);
    if context.is_open() != original {
        return Err("Hidden Drawer IME state could not be restored".into());
    }
    Ok(if changed {
        "toggle-and-restore"
    } else {
        "provider-declined-state-preserved"
    })
}
#[cfg(not(windows))]
pub(super) fn native_probe(_frame: &eframe::Frame) -> Result<&'static str, String> {
    Ok("provider-declined-state-preserved")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestDirectory, click, frame, tiny_spec};
    #[test]
    fn composition_and_commit_events_suppress_drawing_shortcuts_without_touching_history() {
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None);
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let selected = app.selected;
        let key = |key, pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("drawer-ime-composing"), true));
        frame(&ctx, &mut app, vec![key(egui::Key::N, true)]);
        assert_eq!(app.selected, selected);
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::Ime(egui::ImeEvent::Commit("한".into())),
                key(egui::Key::W, true),
            ],
        );
        assert_eq!(app.selected, selected);
        frame(&ctx, &mut app, vec![key(egui::Key::E, true)]);
        assert_eq!(app.selected, BrushTip::Eraser);
        frame(&ctx, &mut app, vec![key(egui::Key::E, false)]);
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
    }
    #[test]
    fn local_language_ime_preferences_warning_restore_cancel_backup_and_restart() {
        let root = TestDirectory::new();
        let storage = UserStorage::at(root.0.join("user")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Drawer::new(&ctx, None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        app.create_canvas(tiny_spec()).unwrap();
        frame(&ctx, &mut app, vec![]);
        let before = app.documents[0].snapshot();
        let history = app.documents[0].history_counts();
        let saved = app.workspaces.store.current.preferences.clone();
        let mut draft = saved.clone();
        draft.language = Language::English;
        draft.text_ime = ImeMode::Open;
        draft.number_ime = ImeMode::Closed;
        draft.suppressed.extend(Confirmation::ALL);
        app.preferences.open = true;
        app.preferences.draft = Some(draft.clone());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let r = app.preferences.cancel.unwrap();
        click(&ctx, &mut app, r);
        assert_eq!(app.workspaces.store.current.preferences, saved);
        app.preferences.open = true;
        app.preferences.draft = Some(draft.clone());
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let r = app.preferences.apply.unwrap();
        click(&ctx, &mut app, r);
        assert_eq!(app.workspaces.store.current.preferences, draft);
        let restarted = Drawer::new(&egui::Context::default(), None)
            .with_storage(UserStorage::at(storage.root().to_path_buf()).unwrap());
        assert_eq!(restarted.workspaces.store.current.preferences, draft);
        app.open_tutorial();
        assert_eq!(app.workspaces.store.current.help.topic, "tutorial-en");
        assert!(drawer_core::help::find("tutorial-en").is_some());
        assert!(tool_hint(Language::Korean, BrushTip::Eraser).contains("지우개"));
        app.workspaces.store.current.help.visible = false;
        app.preferences.open = true;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        let r = app
            .preferences
            .restore_one
            .iter()
            .find(|(w, _)| *w == Confirmation::ResetProperties)
            .unwrap()
            .1;
        click(&ctx, &mut app, r);
        assert!(
            !app.preferences
                .draft
                .as_ref()
                .unwrap()
                .suppressed
                .contains(&Confirmation::ResetProperties)
        );
        assert!(
            app.preferences
                .draft
                .as_ref()
                .unwrap()
                .suppressed
                .contains(&Confirmation::ResetLayout)
        );
        let r = app.preferences.restore_all.unwrap();
        click(&ctx, &mut app, r);
        assert!(
            app.preferences
                .draft
                .as_ref()
                .unwrap()
                .suppressed
                .is_empty()
        );
        let r = app.preferences.apply.unwrap();
        click(&ctx, &mut app, r);
        assert!(
            app.workspaces
                .store
                .current
                .preferences
                .suppressed
                .is_empty()
        );
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), history);
        assert_eq!(
            drawer_core::workspaces::Snapshot::read(
                &storage.root().join("settings/workspace-before-apply.json")
            )
            .unwrap()
            .preferences,
            draft
        );
    }
}
