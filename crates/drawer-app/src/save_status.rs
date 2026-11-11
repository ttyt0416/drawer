use super::*;
#[derive(Default)]
pub(super) struct Panel {
    pub close: [Option<egui::Rect>; 3],
}
#[path = "save_status_verify.rs"]
pub(super) mod verify;
