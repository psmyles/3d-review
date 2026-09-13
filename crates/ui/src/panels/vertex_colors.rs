//! Vertex Color tool options: which channels of the mesh's vertex-color
//! attribute to visualize. RGB shows the color channels, Alpha shows the alpha
//! as 0..1 greyscale, and RGB+A uses alpha as surface opacity.

use review_render::VertexColorMode;

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::{UiState, VertexColorPanelState};
use crate::widgets::{Tip, labeled_combo, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsVertexColors;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "vertex_colors", |ui| {
        color_mode_row(ui, &mut state.vertex_colors.mode);
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.vertex_colors = VertexColorPanelState::default();
    }
}

/// "Color Mode" row: a dropdown choosing which vertex-color channels the view
/// shows.
fn color_mode_row(ui: &mut egui::Ui, mode: &mut VertexColorMode) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::COLOR_MODE)
            .describe(keys::ui_panels::COLOR_MODE_DESCRIPTION)
            .page(PAGE),
        "vertex_color_mode",
        labels::vertex_color_mode(*mode),
        |ui| {
            for choice in [
                VertexColorMode::Rgb,
                VertexColorMode::Alpha,
                VertexColorMode::RgbAlpha,
            ] {
                ui.selectable_value(mode, choice, labels::vertex_color_mode(choice));
            }
        },
    );
}
