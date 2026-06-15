//! The model-stats overlay: a compact, fixed-width grid of measured counts.
//!
//! Every value shown is a real measured number carried through import in
//! [`review_model::ModelStats`] (invariant 5) — never a placeholder.

use crate::state::UiState;
use crate::theme::{color, font, size};
use crate::widgets::mono_label;

pub(crate) fn stats_grid(ui: &mut egui::Ui, state: &UiState) {
    let stats = &state.stats;
    ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;
    stat_row(ui, "Draws", &stats.draw_count.to_string());
    stat_row(ui, "Polys", &stats.polygon_count.to_string());
    stat_row(ui, "Tris", &stats.triangle_count.to_string());
    stat_row(ui, "Verts", &stats.vertex_count.to_string());
    stat_row(ui, "UV Sets", &stats.uv_set_count.to_string());
    stat_row(ui, "FPS", &format!("{:.0}", state.fps));
}

/// One stats row: label hugs the left edge, value right-aligns against the
/// panel's right edge so the numeric column reads as a tidy block.
fn stat_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(mono_label(label, font::STATS, color::TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(mono_label(value, font::STATS, color::TEXT_VALUE));
        });
    });
}
