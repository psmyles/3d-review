//! UV Checker tool options: which built-in checker, how densely it tiles, and —
//! for multi-UV-set models — which UV channel to visualize.

use review_render::CheckerTexture;

use crate::state::{CHECKER_TILING_MAX, CHECKER_TILING_MIN, UiState, UvCheckerPanelState};
use crate::theme::size;
use crate::widgets::{compact_combo, table_label_cell, wide_reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    let uv_set_count = state.stats.uv_set_count;
    checker_texture_row(ui, &mut state.uv_checker.texture);
    ui.add_space(size::PANEL_ROW_GAP);
    checker_tiling_row(ui, &mut state.uv_checker);
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

/// "Checker Tiling" row: an integer slider (1..=16) paired with a text field.
/// The slider updates the text mirror live; the text field is re-sanitized to a
/// clamped integer when editing finishes (focus loss or Enter).
fn checker_tiling_row(ui: &mut egui::Ui, state: &mut UvCheckerPanelState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "Checker Tiling");
        let slider_w = (control_w - size::PANEL_TILING_TEXT_W - size::PANEL_COL_GAP).max(0.0);

        ui.spacing_mut().slider_width = slider_w;
        let slider = ui.add(
            egui::Slider::new(&mut state.tiling, CHECKER_TILING_MIN..=CHECKER_TILING_MAX)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        if slider.changed() {
            state.tiling_text = state.tiling.to_string();
        }

        ui.add_space(size::PANEL_COL_GAP);
        let field = ui.add_sized(
            egui::vec2(size::PANEL_TILING_TEXT_W, size::PANEL_ROW_H),
            egui::TextEdit::singleline(&mut state.tiling_text)
                .horizontal_align(egui::Align::Center),
        );
        // Commit only when editing ends: an empty / non-numeric entry reverts to
        // the current value, anything else is clamped into range.
        if field.lost_focus() {
            let committed = state
                .tiling_text
                .trim()
                .parse::<u32>()
                .unwrap_or(state.tiling)
                .clamp(CHECKER_TILING_MIN, CHECKER_TILING_MAX);
            state.tiling = committed;
            state.tiling_text = committed.to_string();
        }
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
