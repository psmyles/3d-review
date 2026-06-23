//! Vertex Color tool options: which channels of the mesh's vertex-color
//! attribute to visualize. RGB shows the color channels, Alpha shows the alpha
//! as 0..1 greyscale, and RGB+A uses alpha as surface opacity.

use review_render::VertexColorMode;

use crate::state::{UiState, VertexColorPanelState};
use crate::widgets::{labeled_combo, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "vertex_colors", |ui| {
        color_mode_row(ui, &mut state.vertex_colors.mode);
    });
    if reset_button(ui).clicked() {
        state.vertex_colors = VertexColorPanelState::default();
    }
}

/// "Color Mode" row: a dropdown choosing which vertex-color channels the view
/// shows.
fn color_mode_row(ui: &mut egui::Ui, mode: &mut VertexColorMode) {
    labeled_combo(
        ui,
        "Color Mode",
        "vertex_color_mode",
        color_mode_label(*mode),
        |ui| {
            ui.selectable_value(mode, VertexColorMode::Rgb, "RGB channel");
            ui.selectable_value(mode, VertexColorMode::Alpha, "Alpha channel");
            ui.selectable_value(mode, VertexColorMode::RgbAlpha, "RGB+A channel");
        },
    );
}

fn color_mode_label(mode: VertexColorMode) -> &'static str {
    match mode {
        VertexColorMode::Rgb => "RGB channel",
        VertexColorMode::Alpha => "Alpha channel",
        VertexColorMode::RgbAlpha => "RGB+A channel",
    }
}
