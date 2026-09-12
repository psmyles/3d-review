//! Ambient-occlusion options: the sample radius, the strength of the darkening,
//! the thickness heuristic, and the sampling quality. (The implementation is
//! GTAO; the UI keeps the user-facing "Ambient Occlusion" name.)
//!
//! All write straight into [`UiState::gtao`], which the viewport callback reads
//! each frame; there is no separate commit step. The on/off master toggle is the
//! AO button in the status bar (see `status_bar.rs`), not a row here.
//!
//! `radius` is a **multiplier** over the radius the renderer derives from what the
//! viewport is showing, so it carries taste rather than the scene's scale — which
//! is why the row prints the resolved distance next to it. Without that the same
//! number means centimetres on a prop and metres in a building, and there is no way
//! to tell from the panel which you have.

use review_render::{GtaoQuality, GtaoSettings};

use crate::state::{
    UiState,
    range::{
        AO_INTENSITY_MAX, AO_INTENSITY_MIN, AO_RADIUS_MAX, AO_RADIUS_MIN, AO_THICKNESS_MAX,
        AO_THICKNESS_MIN,
    },
};
use crate::theme::color;
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
        radius_readout(ui, state.gtao_world_radius);
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

/// The distance the Radius multiplier currently resolves to, under its row.
///
/// Formatted by magnitude rather than with a fixed precision: the viewer spans
/// millimetre props to kilometre landscapes, so no single number of decimals reads
/// well across it — `0.24 m` and `1.8 km` are both three significant figures, while
/// a fixed two decimals would print `0.00 m` on one and `1800.00 m` on the other.
fn radius_readout(ui: &mut egui::Ui, metres: f32) {
    ui.label("");
    ui.label(
        egui::RichText::new(format_distance(metres))
            .color(color::TEXT_MUTED)
            .small(),
    );
    ui.end_row();
}

fn format_distance(metres: f32) -> String {
    let m = metres.max(0.0);
    if m >= 1000.0 {
        format!("{:.2} km", m / 1000.0)
    } else if m >= 1.0 {
        format!("{m:.2} m")
    } else if m >= 0.01 {
        format!("{:.1} cm", m * 100.0)
    } else {
        format!("{:.1} mm", m * 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::format_distance;

    /// The viewer spans several orders of magnitude, so the readout has to stay
    /// legible at each of them rather than bottoming out at `0.00`.
    #[test]
    fn a_distance_reads_in_the_unit_that_suits_its_magnitude() {
        assert_eq!(format_distance(2400.0), "2.40 km");
        assert_eq!(format_distance(12.5), "12.50 m");
        assert_eq!(format_distance(0.24), "24.0 cm");
        assert_eq!(format_distance(0.004), "4.0 mm");
    }
}
