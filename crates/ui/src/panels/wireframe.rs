//! Wireframe tool options: color and a reset to defaults.

use crate::docs::Page;
use crate::keys;
use crate::state::{UiState, WireframePanelState};
use crate::theme::color;
use crate::widgets::{Tip, color_swatch_row, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsWireframe;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "wireframe", |ui| {
        color_swatch_row(
            ui,
            Tip::new(keys::ui_panels::WIREFRAME_COLOR)
                .describe(keys::ui_panels::WIREFRAME_COLOR_DESCRIPTION)
                .page(PAGE),
            &mut state.wireframe.color,
            &color::WIREFRAME_SWATCHES,
        );
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.wireframe = WireframePanelState::default();
    }
}
