//! Screen-space ambient occlusion options: the sample radius, the strength of the
//! darkening, and the depth bias that suppresses self-occlusion acne.
//!
//! All write straight into [`UiState::ssao`], which the viewport callback reads
//! each frame; there is no separate commit step. The SSAO on/off master toggle is
//! the AO button in the status bar (see `status_bar.rs`), not a row here. `radius`
//! and `bias` are fractions of the framed model's bounding-sphere radius, so the
//! look stays consistent across model scales (the renderer scales them by the live
//! scene radius).

use review_render::SsaoSettings;

use crate::state::UiState;
use crate::theme::size;
use crate::widgets::{labeled_slider, wide_reset_button};

/// Inclusive slider ranges. Radius/bias are scene-radius fractions; intensity
/// scales the raw occlusion.
const RADIUS_MIN: f32 = 0.02;
const RADIUS_MAX: f32 = 1.0;
const INTENSITY_MIN: f32 = 0.0;
const INTENSITY_MAX: f32 = 2.0;
const BIAS_MIN: f32 = 0.0;
const BIAS_MAX: f32 = 0.1;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    labeled_slider(
        ui,
        "Radius",
        &mut state.ssao.radius,
        RADIUS_MIN..=RADIUS_MAX,
    );
    ui.add_space(size::PANEL_ROW_GAP);

    labeled_slider(
        ui,
        "Intensity",
        &mut state.ssao.intensity,
        INTENSITY_MIN..=INTENSITY_MAX,
    );
    ui.add_space(size::PANEL_ROW_GAP);

    labeled_slider(ui, "Bias", &mut state.ssao.bias, BIAS_MIN..=BIAS_MAX);
    ui.add_space(size::PANEL_ACTION_GAP);

    if wide_reset_button(ui).clicked() {
        // The on/off state is owned by the status-bar toggle, so reset only the
        // panel's own options and leave `enabled` untouched.
        state.ssao = SsaoSettings {
            enabled: state.ssao.enabled,
            ..SsaoSettings::default()
        };
    }
}
