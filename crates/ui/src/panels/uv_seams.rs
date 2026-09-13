//! UV Seams tool options: the edge color, and — for multi-UV-set models — which
//! UV channel the seam test reads.
//!
//! There is deliberately no thickness control: the scene's line pipeline draws at
//! a fixed 1px hardware width, so the color is the whole of what separates a seam
//! from the wireframe edge underneath it.

use crate::docs::Page;
use crate::keys;
use crate::state::{UiState, UvSeamPanelState};
use crate::theme::color;
use crate::widgets::{Tip, color_swatch_row, labeled_combo, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsUvSeams;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    let uv_set_count = state.stats.uv_set_count;
    panel_grid(ui, "uv_seams", |ui| {
        color_swatch_row(
            ui,
            Tip::new(keys::ui_panels::LINE_COLOR)
                .describe(keys::ui_panels::LINE_COLOR_DESCRIPTION)
                .page(PAGE),
            &mut state.uv_seams.color,
            &color::UV_SEAM_SWATCHES,
        );
        // Only meaningful — and only shown — when the model carries more than one
        // UV set, matching the UV Checker panel's channel row.
        if uv_set_count > 1 {
            seam_channel_row(ui, &mut state.uv_seams.uv_channel, uv_set_count);
        }
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.uv_seams = UvSeamPanelState::default();
    }
}

/// "Model UV channel" row: a dropdown selecting which of the model's UV sets is
/// tested for seams. Only shown for models with more than one set.
fn seam_channel_row(ui: &mut egui::Ui, uv_channel: &mut u32, uv_set_count: usize) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::UV_CHANNEL)
            .describe(keys::ui_panels::UV_CHANNEL_DESCRIPTION)
            .page(PAGE),
        "uv_seam_channel",
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
