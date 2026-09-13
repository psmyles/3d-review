//! Face- and vertex-normal tool options. Both share [`NormalPanelState`]: a line
//! length slider and a line color swatch row, plus a reset to that view's
//! defaults.

use crate::docs::Page;
use crate::keys;
use crate::state::{
    DEFAULT_NORMAL_LENGTH, NormalPanelState, UiState,
    range::{NORMAL_LENGTH_MAX, NORMAL_LENGTH_MIN},
};
use crate::theme::color;
use crate::widgets::{Tip, color_swatch_row, labeled_slider_with_value, panel_footer, panel_grid};

pub(super) fn face_body(ui: &mut egui::Ui, state: &mut UiState) {
    normal_body(
        ui,
        &mut state.face_normals,
        color::FACE_NORMAL_DEFAULT,
        Page::PanelsFaceNormals,
    );
}

pub(super) fn vertex_body(ui: &mut egui::Ui, state: &mut UiState) {
    normal_body(
        ui,
        &mut state.vertex_normals,
        color::VERTEX_NORMAL_DEFAULT,
        Page::PanelsVertexNormals,
    );
}

/// Both normal panels share this body; only the state they edit, the colour they
/// reset to, and the manual page their `?` opens differ.
fn normal_body(
    ui: &mut egui::Ui,
    normals: &mut NormalPanelState,
    default_color: egui::Color32,
    page: Page,
) {
    panel_grid(ui, "normals", |ui| {
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::NORMAL_LENGTH)
                .describe(keys::ui_panels::NORMAL_LENGTH_DESCRIPTION)
                .page(page),
            &mut normals.length,
            NORMAL_LENGTH_MIN..=NORMAL_LENGTH_MAX,
            3,
        );
        color_swatch_row(
            ui,
            Tip::new(keys::ui_panels::LINE_COLOR)
                .describe(keys::ui_panels::LINE_COLOR_DESCRIPTION)
                .page(page),
            &mut normals.color,
            &color::NORMAL_SWATCHES,
        );
    });
    ui.separator();
    if panel_footer(ui, page).clicked() {
        normals.length = DEFAULT_NORMAL_LENGTH;
        normals.color = default_color;
    }
}
