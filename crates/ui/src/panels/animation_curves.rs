//! Animation Curves tool options: the shape of the line between the frame dots,
//! its width and opacity, and the dots' size, plus a reset to defaults.
//!
//! Opened by right-clicking the transport's animation-curve toggle. Only the
//! shape rebuilds the curves; the rest are uniforms the renderer applies at
//! draw time, so a slider drag costs nothing per tick.

use crate::docs::Page;
use crate::keys;
use crate::state::{
    CurveShape, CurvesPanelState, UiState,
    range::{
        CURVE_DOT_SIZE_MAX, CURVE_DOT_SIZE_MIN, CURVE_OPACITY_MAX, CURVE_OPACITY_MIN,
        CURVE_WIDTH_MAX, CURVE_WIDTH_MIN,
    },
};
use crate::widgets::{Tip, labeled_combo, labeled_slider_with_value, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsAnimationCurves;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    let curves = &mut state.curves;
    panel_grid(ui, "animation_curves", |ui| {
        labeled_combo(
            ui,
            Tip::new(keys::ui_panels::CURVE_SHAPE)
                .describe(keys::ui_panels::CURVE_SHAPE_DESCRIPTION)
                .page(PAGE),
            "animation_curve_shape",
            curves.shape.label(),
            |ui| {
                for shape in CurveShape::ALL {
                    ui.selectable_value(&mut curves.shape, shape, shape.label());
                }
            },
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::CURVE_WIDTH)
                .describe(keys::ui_panels::CURVE_WIDTH_DESCRIPTION)
                .page(PAGE),
            &mut curves.width,
            CURVE_WIDTH_MIN..=CURVE_WIDTH_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::CURVE_OPACITY)
                .describe(keys::ui_panels::CURVE_OPACITY_DESCRIPTION)
                .page(PAGE),
            &mut curves.opacity,
            CURVE_OPACITY_MIN..=CURVE_OPACITY_MAX,
            2,
        );
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::CURVE_DOT_SIZE)
                .describe(keys::ui_panels::CURVE_DOT_SIZE_DESCRIPTION)
                .page(PAGE),
            &mut curves.dot_size,
            CURVE_DOT_SIZE_MIN..=CURVE_DOT_SIZE_MAX,
            1,
        );
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.curves = CurvesPanelState::default();
    }
}
