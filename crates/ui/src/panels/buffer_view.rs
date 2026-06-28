//! Buffers tool options (behind the Buffers button): which single material /
//! geometry buffer the filled faces show for data inspection — base color, the
//! world / geometric normal, the raw normal map, tangents, roughness, metallic,
//! AO, emission, opacity or UVs. The renderer shows the chosen buffer flat
//! (bypassing lighting + tone mapping) so the displayed pixel is the value itself.

use review_render::BufferView;

use crate::state::UiState;
use crate::widgets::{labeled_combo, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "buffer_view", |ui| {
        view_row(ui, &mut state.debug.buffer_view);
    });
    ui.separator();
    if reset_button(ui).clicked() {
        state.debug.buffer_view = BufferView::default();
    }
}

/// "Buffer" row: a dropdown choosing which buffer the filled faces display.
fn view_row(ui: &mut egui::Ui, view: &mut BufferView) {
    labeled_combo(ui, "Buffer", "buffer_view_combo", view.label(), |ui| {
        for option in BufferView::ALL {
            ui.selectable_value(view, option, option.label());
        }
    });
}
