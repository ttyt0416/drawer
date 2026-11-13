//! Local display strings only. Stable command IDs and document/user text are untouched.
use drawer_core::preferences::Language;
use std::{borrow::Cow, cell::Cell, collections::HashMap, sync::OnceLock};
thread_local! {static LANGUAGE:Cell<Language>=const {Cell::new(Language::English)};}
pub(super) struct Scope(Language);
impl Scope {
    pub fn enter(language: Language) -> Self {
        Self(LANGUAGE.replace(language))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        LANGUAGE.set(self.0);
    }
}
fn catalog() -> &'static HashMap<String, String> {
    static WORDS: OnceLock<HashMap<String, String>> = OnceLock::new();
    WORDS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../locales/ko.json")).unwrap_or_default()
    })
}
pub(super) fn text(value: &str) -> Cow<'_, str> {
    if LANGUAGE.get() == Language::Korean
        && let Some(korean) = catalog().get(value)
    {
        Cow::Borrowed(korean.as_str())
    } else {
        Cow::Borrowed(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;
    fn rendered(ctx: &egui::Context, app: &mut crate::Drawer) -> Vec<String> {
        painted(ctx, app, false)
    }
    fn painted(ctx: &egui::Context, app: &mut crate::Drawer, commands: bool) -> Vec<String> {
        fn visit(shape: &egui::epaint::Shape, text: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(t) => text.push(t.galley.job.text.clone()),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = Vec::new();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 700.),
                )),
                ..Default::default()
            },
            |ui| {
                if commands {
                    let _language = Scope::enter(app.workspaces.store.current.preferences.language);
                    app.document_controls(ui);
                } else {
                    app.show(ui);
                }
            },
        );
        for shape in &output.shapes {
            visit(&shape.shape, &mut text);
        }
        text
    }
    #[test]
    fn actual_language_switch_scaling_and_user_document_name_leave_document_unchanged() {
        let ctx = egui::Context::default();
        let mut app = crate::Drawer::new(&ctx, None);
        app.simple_view = true;
        app.create_canvas(crate::tests::tiny_spec()).unwrap();
        app.documents[0].title = "Save".into();
        app.workspaces.store.current.preferences.language = Language::English;
        for _ in 0..3 {
            rendered(&ctx, &mut app);
        }
        let before = app.documents[0].snapshot();
        let counts = app.documents[0].history_counts();
        let english = rendered(&ctx, &mut app);
        assert!(english.iter().any(|s| s.contains("All tools and commands")));
        app.workspaces.store.current.preferences.language = Language::Korean;
        let mut korean = Vec::new();
        for _ in 0..3 {
            korean = rendered(&ctx, &mut app);
        }
        assert!(korean.iter().any(|s| s.contains("모든 도구와 명령")));
        let command_context = egui::Context::default();
        let mut commands = Vec::new();
        for _ in 0..3 {
            commands = painted(&command_context, &mut app, true);
        }
        assert!(
            commands.iter().any(|s| s.starts_with("저장  ")),
            "Save command did not localize while keeping its shortcut: {commands:?}"
        );
        assert!(
            korean.iter().any(|s| s.contains("Save")),
            "User document name was translated"
        );
        ctx.set_zoom_factor(1.75);
        for _ in 0..3 {
            rendered(&ctx, &mut app);
        }
        assert_eq!(app.documents[0].snapshot(), before);
        assert_eq!(app.documents[0].history_counts(), counts);
        app.workspaces.store.current.preferences.language = Language::English;
        let mut english = Vec::new();
        for _ in 0..3 {
            english = rendered(&ctx, &mut app);
        }
        assert!(english.iter().any(|s| s.contains("All tools and commands")));
    }
    #[test]
    fn runtime_scope_english_fallback_and_unicode_preserve_user_content() {
        let words: HashMap<String, String> =
            serde_json::from_str(include_str!("../../../locales/ko.json")).unwrap();
        assert!(
            words.len() >= 200
                && words
                    .iter()
                    .all(|(a, b)| !a.trim().is_empty() && !b.trim().is_empty())
        );
        assert_eq!(text("Save"), "Save");
        {
            let _scope = Scope::enter(Language::Korean);
            assert_eq!(text("Save"), "저장");
            assert_eq!(text("사용자 문서 🎨"), "사용자 문서 🎨");
            assert_eq!(text("Untranslated diagnostic"), "Untranslated diagnostic");
            {
                let _nested = Scope::enter(Language::English);
                assert_eq!(text("Save"), "Save");
            }
            assert_eq!(text("Save"), "저장");
        }
        assert_eq!(text("Save"), "Save");
    }
}
