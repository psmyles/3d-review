//! Ambient-occlusion options: the sample radius, the strength of the darkening,
//! the thickness heuristic, and the sampling quality. (The implementation is
//! GTAO; the UI keeps the user-facing "Ambient Occlusion" name.)
//!
//! All write straight into [`UiState::gtao`], which the viewport callback reads
//! each frame; there is no separate commit step. The on/off master toggle is the
//! AO button in the status bar (see `status_bar.rs`), not a row here. `radius` is a
//! fraction of the framed model's bounding-sphere radius, so the look stays
//! consistent across model scales (the renderer scales it by the live scene
//! radius).

use review_render::{GtaoQuality, GtaoSettings};

use crate::state::UiState;
use crate::widgets::{labeled_combo, labeled_slider_with_value, panel_grid, reset_button};

/// Inclusive slider ranges. Radius is a scene-radius fraction; intensity is the
/// power on the GTAO visibility; thickness is the 0..1 see-through heuristic.
const RADIUS_MIN: f32 = 0.02;
const RADIUS_MAX: f32 = 1.0;
const INTENSITY_MIN: f32 = 0.0;
const INTENSITY_MAX: f32 = 2.0;
const THICKNESS_MIN: f32 = 0.0;
const THICKNESS_MAX: f32 = 1.0;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "gtao", |ui| {
        labeled_slider_with_value(
            ui,
            "Radius",
            &mut state.gtao.radius,
            RADIUS_MIN..=RADIUS_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            "Intensity",
            &mut state.gtao.intensity,
            INTENSITY_MIN..=INTENSITY_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            "Thickness",
            &mut state.gtao.thickness,
            THICKNESS_MIN..=THICKNESS_MAX,
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
