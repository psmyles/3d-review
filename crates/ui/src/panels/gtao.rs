//! Ambient-occlusion options: the sample radius, the strength of the darkening,
//! the thickness heuristic, and the sampling quality. (The implementation is
//! GTAO; the UI keeps the user-facing "Ambient Occlusion" name.)
//!
//! All write straight into [`UiState::gtao`], which the viewport callback reads
//! each frame; there is no separate commit step. The on/off master toggle is the
//! AO button in the status bar (see `status_bar.rs`), not a row here.
//!
//! `radius` is a **multiplier** over the radius the renderer derives from what the
//! viewport is showing, so it carries taste rather than the scene's scale.

use review_render::{GtaoQuality, GtaoSettings};

use crate::state::{
    UiState,
    range::{
        AO_INTENSITY_MAX, AO_INTENSITY_MIN, AO_RADIUS_MAX, AO_RADIUS_MIN, AO_THICKNESS_MAX,
        AO_THICKNESS_MIN,
    },
};
use crate::widgets::{labeled_combo, labeled_slider_with_value, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "gtao", |ui| {
        labeled_slider_with_value(
            ui,
            "Radius",
            &mut state.gtao.radius,
            AO_RADIUS_MIN..=AO_RADIUS_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            "Intensity",
            &mut state.gtao.intensity,
            AO_INTENSITY_MIN..=AO_INTENSITY_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            "Thickness",
            &mut state.gtao.thickness,
            AO_THICKNESS_MIN..=AO_THICKNESS_MAX,
            2,
        );
        labeled_combo(
            ui,
            "Quality",
            "gtao_quality",
            state.gtao.quality.label(),
            |ui| {
                for quality in GtaoQuality::ALL {
                    ui.selectable_value(&mut state.gtao.quality, quality, quality.label());
                }
            },
        );
    });

    ui.separator();
    if reset_button(ui).clicked() {
        // The on/off state is owned by the status-bar toggle, so reset only the
        // panel's own options and leave `enabled` untouched.
        state.gtao = GtaoSettings {
            enabled: state.gtao.enabled,
            ..GtaoSettings::default()
        };
    }
}
