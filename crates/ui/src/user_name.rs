//! Preferences > User Name: the box the name review comments are signed with is
//! changed in. It edits a copy, so Cancel - or Escape, or a click outside it -
//! leaves the name as it was; Save hands the new one to the comments state, and
//! `app` writes it to the settings file from there, exactly as it does the name
//! the composer asks for on a first comment.
//!
//! An `egui::Modal`, as the About box is: egui's own primitive for a window that
//! blocks the rest of the chrome until it is answered.

use crate::keys;
use crate::state::UiState;
use crate::theme::size;

/// Whether the User Name box is up, and the name being typed in it.
#[derive(Debug, Clone, Default)]
pub struct UserNameState {
    pub open: bool,
    /// The name as typed, copied from the current one when the box opened.
    pub draft: String,
    /// Set once the text field has taken keyboard focus.
    focused: bool,
}

impl UserNameState {
    /// Open the box on `current`, the name comments are signed with now.
    pub fn open(&mut self, current: &str) {
        self.open = true;
        self.draft = current.to_owned();
        self.focused = false;
    }
}

/// Draw the User Name box while it is open. Save (or Enter) keeps the name typed,
/// once there is one; Cancel, Escape and a click outside the box drop it.
pub(crate) fn draw(ctx: &egui::Context, state: &mut UiState) {
    if !state.user_name.open {
        return;
    }
    let dialog = &mut state.user_name;
    let mut save = false;
    let mut cancel = false;
    let response = egui::Modal::new(egui::Id::new("user_name_modal")).show(ctx, |ui| {
        ui.set_width(size::USER_NAME_WIDTH);
        ui.heading(keys::ui_comments::USER_NAME_TITLE);
        ui.label(keys::ui_comments::USER_NAME_INTRO);
        let field = ui.add(
            egui::TextEdit::singleline(&mut dialog.draft)
                .hint_text(keys::ui_comments::COMPOSER_NAME_HINT)
                .desired_width(f32::INFINITY),
        );
        if !dialog.focused {
            field.request_focus();
            dialog.focused = true;
        }
        let named = !dialog.draft.trim().is_empty();
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.separator();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel = ui.button(keys::ui_comments::USER_NAME_CANCEL).clicked();
            save = ui
                .add_enabled(named, egui::Button::new(keys::ui_comments::USER_NAME_SAVE))
                .clicked();
        });
        save |= named && entered;
    });
    if save {
        state.comments.author = state.user_name.draft.trim().to_owned();
        state.user_name.open = false;
    } else if cancel || response.should_close() {
        state.user_name.open = false;
    }
}
