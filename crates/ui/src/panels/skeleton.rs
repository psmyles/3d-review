//! Skeleton overlay tool options: a size multiplier for the octahedral bones and
//! their joint markers, plus a bone color swatch row.
//!
//! The selected-bone highlight is deliberately *not* configurable here — it reuses
//! the viewport's selection color so a selected bone reads the same as a selected
//! mesh part (see `sync_debug_state`).

use crate::docs::Page;
use crate::keys;
use crate::state::{
    DEFAULT_SKELETON_SCALE, UiState,
    range::{SKELETON_SCALE_MAX, SKELETON_SCALE_MIN},
};
use crate::theme::color;
use crate::widgets::{Tip, color_swatch_row, labeled_slider_with_value, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsSkeleton;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "skeleton", |ui| {
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::BONE_SIZE)
                .describe(keys::ui_panels::BONE_SIZE_DESCRIPTION)
                .page(PAGE),
            &mut state.skeleton.scale,
            SKELETON_SCALE_MIN..=SKELETON_SCALE_MAX,
            2,
        );
        color_swatch_row(
            ui,
            Tip::new(keys::ui_panels::BONE_COLOR)
                .describe(keys::ui_panels::BONE_COLOR_DESCRIPTION)
                .page(PAGE),
            &mut state.skeleton.color,
            &color::SKELETON_SWATCHES,
        );
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.skeleton.scale = DEFAULT_SKELETON_SCALE;
        state.skeleton.color = color::SKELETON_DEFAULT;
    }
}
