//! Wireframe tool options: line width, color and a reset to defaults.

use crate::docs::Page;
use crate::keys;
use crate::state::{
    UiState, WireframePanelState,
    range::{WIREFRAME_WIDTH_MAX, WIREFRAME_WIDTH_MIN},
};
use crate::theme::color;
use crate::widgets::{Tip, color_swatch_row, labeled_slider_with_value, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsWireframe;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "wireframe", |ui| {
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::WIREFRAME_WIDTH)
                .describe(keys::ui_panels::WIREFRAME_WIDTH_DESCRIPTION)
                .page(PAGE),
            &mut state.wireframe.width,
            WIREFRAME_WIDTH_MIN..=WIREFRAME_WIDTH_MAX,
            2,
        );
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
