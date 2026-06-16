//! Vertex Color tool options: which channels of the mesh's vertex-color
//! attribute to visualize. RGB shows the color channels, Alpha shows the alpha
//! as 0..1 greyscale, and RGB+A uses alpha as surface opacity.

use review_render::VertexColorMode;

use crate::state::{UiState, VertexColorPanelState};
use crate::theme::size;
use crate::widgets::{compact_combo, table_label_cell, wide_reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    color_mode_row(ui, &mut state.vertex_colors.mode);
    ui.add_space(size::PANEL_ACTION_GAP);
    if wide_reset_button(ui).clicked() {
        state.vertex_colors = VertexColorPanelState::default();
    }
}

/// "Color Mode" row: a dropdown choosing which vertex-color channels the view
/// shows.
fn color_mode_row(ui: &mut egui::Ui, mode: &mut VertexColorMode) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "Color Mode");
        compact_combo(
            ui,
            "vertex_color_mode",
            control_w,
            color_mode_label(*mode),
            |ui| {
                ui.selectable_value(mode, VertexColorMode::Rgb, "RGB channel");
                ui.selectable_value(mode, VertexColorMode::Alpha, "Alpha channel");
                ui.selectable_value(mode, VertexColorMode::RgbAlpha, "RGB+A channel");
            },
        );
    });
}

fn color_mode_label(mode: VertexColorMode) -> &'static str {
    match mode {
        VertexColorMode::Rgb => "RGB channel",
        VertexColorMode::Alpha => "Alpha channel",
        VertexColorMode::RgbAlpha => "RGB+A channel",
    }
}
