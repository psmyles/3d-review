//! Bloom (HDR glow) options: the brightness threshold above which a pixel glows
//! and the intensity of the glow added back to the scene.
//!
//! Both write straight into [`UiState::bloom`], which the viewport callback reads
//! each frame; there is no separate commit step. The bloom on/off master toggle is
//! the bloom button in the status bar (see `status_bar.rs`), not a row here.

use review_render::BloomSettings;

use crate::state::UiState;
use crate::widgets::{labeled_slider_with_value, panel_grid, reset_button};

/// Inclusive ranges for the bloom sliders. Threshold is in linear-HDR luminance
/// (1.0 = "only brighter-than-white highlights glow"); intensity scales the glow.
const THRESHOLD_MIN: f32 = 0.0;
const THRESHOLD_MAX: f32 = 3.0;
const INTENSITY_MIN: f32 = 0.0;
const INTENSITY_MAX: f32 = 2.0;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "bloom", |ui| {
        labeled_slider_with_value(
            ui,
            "Threshold",
            &mut state.bloom.threshold,
            THRESHOLD_MIN..=THRESHOLD_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            "Intensity",
            &mut state.bloom.intensity,
            INTENSITY_MIN..=INTENSITY_MAX,
            2,
        );
    });

    ui.separator();
    if reset_button(ui).clicked() {
        // The on/off state is owned by the status-bar toggle, so reset only the
        // panel's own options and leave `enabled` untouched.
        state.bloom = BloomSettings {
            enabled: state.bloom.enabled,
            ..BloomSettings::default()
        };
    }
}
