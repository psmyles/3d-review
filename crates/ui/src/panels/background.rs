//! Viewport-background tool options: the preset swatch row and a reset to default.

use review_render::ViewportBackground;

use crate::docs::Page;
use crate::keys;
use crate::state::UiState;
use crate::widgets::{Tip, background_swatch_row, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsBackground;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "background", |ui| {
        background_swatch_row(
            ui,
            Tip::new(keys::ui_panels::BACKGROUND)
                .describe(keys::ui_panels::BACKGROUND_DESCRIPTION)
                .page(PAGE),
            &mut state.viewport_background,
        );
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.viewport_background = ViewportBackground::default();
    }
}
