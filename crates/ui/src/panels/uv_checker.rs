//! UV Checker tool options: which built-in checker, how densely it tiles, and —
//! for multi-UV-set models — which UV channel to visualize.

use review_render::CheckerTexture;

use crate::state::{
    UiState, UvCheckerPanelState,
    range::{CHECKER_TILING_MAX, CHECKER_TILING_MIN},
};
use crate::widgets::{labeled_combo, labeled_slider_with_value, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    let uv_set_count = state.stats.uv_set_count;
    panel_grid(ui, "uv_checker", |ui| {
        checker_texture_row(ui, &mut state.uv_checker.texture);
        // Integer tiling on the standard slider (0 decimals → integer readout).
        labeled_slider_with_value(
            ui,
            "Checker Tiling",
            &mut state.uv_checker.tiling,
            CHECKER_TILING_MIN..=CHECKER_TILING_MAX,
            0,
        );
        // The channel picker is only meaningful — and only shown — when the model
        // carries more than one UV set.
        if uv_set_count > 1 {
            checker_channel_row(ui, &mut state.uv_checker.uv_channel, uv_set_count);
        }
    });
    ui.separator();
    if reset_button(ui).clicked() {
        state.uv_checker = UvCheckerPanelState::default();
    }
}

/// "Checker Texture" row: a dropdown choosing which built-in checker the UV view
/// samples.
fn checker_texture_row(ui: &mut egui::Ui, texture: &mut CheckerTexture) {
    labeled_combo(
        ui,
        "Checker Texture",
        "uv_checker_texture",
        texture.label(),
        |ui| {
            for choice in [CheckerTexture::Greyscale, CheckerTexture::Color] {
                ui.selectable_value(texture, choice, choice.label());
            }
        },
    );
}

/// "Model UV channel" row: a dropdown selecting which of the model's UV sets the
/// checker view visualizes. Only shown for models with more than one set.
fn checker_channel_row(ui: &mut egui::Ui, uv_channel: &mut u32, uv_set_count: usize) {
    labeled_combo(
        ui,
        "Model UV channel",
        "uv_checker_channel",
        format!("Channel {uv_channel}"),
        |ui| {
            for channel in 0..uv_set_count as u32 {
                ui.selectable_value(uv_channel, channel, format!("Channel {channel}"));
            }
        },
    );
}
