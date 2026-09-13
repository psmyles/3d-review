//! UV Checker tool options: which built-in checker, how densely it tiles, and —
//! for multi-UV-set models — which UV channel to visualize.

use review_render::CheckerTexture;

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::{
    UiState, UvCheckerPanelState,
    range::{CHECKER_TILING_MAX, CHECKER_TILING_MIN},
};
use crate::widgets::{Tip, labeled_combo, labeled_slider_with_value, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsUvChecker;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    let uv_set_count = state.stats.uv_set_count;
    panel_grid(ui, "uv_checker", |ui| {
        checker_texture_row(ui, &mut state.uv_checker.texture);
        // Integer tiling on the standard slider (0 decimals → integer readout).
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::CHECKER_TILING)
                .describe(keys::ui_panels::CHECKER_TILING_DESCRIPTION)
                .page(PAGE),
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
    if panel_footer(ui, PAGE).clicked() {
        state.uv_checker = UvCheckerPanelState::default();
    }
}

/// "Checker Texture" row: a dropdown choosing which built-in checker the UV view
/// samples.
fn checker_texture_row(ui: &mut egui::Ui, texture: &mut CheckerTexture) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::CHECKER_TEXTURE)
            .describe(keys::ui_panels::CHECKER_TEXTURE_DESCRIPTION)
            .page(PAGE),
        "uv_checker_texture",
        labels::checker(*texture),
        |ui| {
            for choice in [CheckerTexture::Greyscale, CheckerTexture::Color] {
                ui.selectable_value(texture, choice, labels::checker(choice));
            }
        },
    );
}

/// "Model UV channel" row: a dropdown selecting which of the model's UV sets the
/// checker view visualizes. Only shown for models with more than one set.
fn checker_channel_row(ui: &mut egui::Ui, uv_channel: &mut u32, uv_set_count: usize) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::UV_CHANNEL)
            .describe(keys::ui_panels::UV_CHANNEL_DESCRIPTION)
            .page(PAGE),
        "uv_checker_channel",
        keys::ui_panels::uv_channel_numbered(f64::from(*uv_channel)),
        |ui| {
            for channel in 0..uv_set_count as u32 {
                ui.selectable_value(
                    uv_channel,
                    channel,
                    keys::ui_panels::uv_channel_numbered(f64::from(channel)),
                );
            }
        },
    );
}
