//! UV Checker tool options: which built-in checker, how densely it tiles, and —
//! for multi-UV-set models — which UV channel to visualize.

use review_render::CheckerTexture;

use crate::state::{CHECKER_TILING_MAX, CHECKER_TILING_MIN, UiState, UvCheckerPanelState};
use crate::theme::size;
use crate::widgets::{
    compact_combo, labeled_slider_with_value, table_label_cell, wide_reset_button,
};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    let uv_set_count = state.stats.uv_set_count;
    checker_texture_row(ui, &mut state.uv_checker.texture);
    ui.add_space(size::PANEL_ROW_GAP);
    // Integer tiling on the standard slider: rail + inline click-to-edit,
    // drag-to-scrub value field (0 decimals, step 1).
    labeled_slider_with_value(
        ui,
        "Checker Tiling",
        &mut state.uv_checker.tiling,
        CHECKER_TILING_MIN..=CHECKER_TILING_MAX,
        0,
        1.0,
    );
    // The channel picker is only meaningful — and only shown — when the model
    // carries more than one UV set.
    if uv_set_count > 1 {
        ui.add_space(size::PANEL_ROW_GAP);
        checker_channel_row(ui, &mut state.uv_checker.uv_channel, uv_set_count);
    }
    ui.add_space(size::PANEL_ACTION_GAP);
    if wide_reset_button(ui).clicked() {
        state.uv_checker = UvCheckerPanelState::default();
    }
}

/// "Checker Texture" row: a dropdown choosing which built-in checker the UV view
/// samples.
fn checker_texture_row(ui: &mut egui::Ui, texture: &mut CheckerTexture) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "Checker Texture");
        compact_combo(
            ui,
            "uv_checker_texture",
            control_w,
            checker_texture_label(*texture),
            |ui| {
                ui.selectable_value(texture, CheckerTexture::Greyscale, "Greyscale");
                ui.selectable_value(texture, CheckerTexture::Color, "Color");
            },
        );
    });
}

/// "Model UV channel" row: a dropdown selecting which of the model's UV sets the
/// checker view visualizes. Only shown for models with more than one set.
fn checker_channel_row(ui: &mut egui::Ui, uv_channel: &mut u32, uv_set_count: usize) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "Model UV channel");
        compact_combo(
            ui,
            "uv_checker_channel",
            control_w,
            format!("Channel {uv_channel}"),
            |ui| {
                for channel in 0..uv_set_count as u32 {
                    ui.selectable_value(uv_channel, channel, format!("Channel {channel}"));
                }
            },
        );
    });
}

fn checker_texture_label(texture: CheckerTexture) -> &'static str {
    match texture {
        CheckerTexture::Greyscale => "Greyscale",
        CheckerTexture::Color => "Color",
    }
}
