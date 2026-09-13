//! Buffers tool options (behind the Buffers button): which single material /
//! geometry buffer the filled faces show for data inspection — base color, the
//! world / geometric normal, the raw normal map, tangents, roughness, metallic,
//! AO, emission, opacity or UVs. The renderer shows the chosen buffer flat
//! (bypassing lighting + tone mapping) so the displayed pixel is the value itself.

use review_render::{BufferView, RoughnessWorkflow};

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::UiState;
use crate::widgets::{Tip, labeled_combo, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsBuffers;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    // Under the Smoothness workflow the Roughness buffer shows (and is labelled)
    // its complement, smoothness — matching the per-material shader inversion.
    let smoothness = shows_smoothness(state);
    panel_grid(ui, "buffer_view", |ui| {
        view_row(ui, &mut state.debug.buffer_view, smoothness);
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.debug.buffer_view = BufferView::default();
    }
}

/// Whether the Roughness buffer should read as **Smoothness**: true when the model
/// has materials and *every* one uses the Smoothness workflow (the Unity-style
/// inverse). A uniform-convention asset then labels the buffer to match what its
/// surfaces actually show; a mixed or roughness asset keeps "Roughness".
fn shows_smoothness(state: &UiState) -> bool {
    !state.materials_snapshot.is_empty()
        && state
            .materials_snapshot
            .iter()
            .all(|material| material.state.workflow == RoughnessWorkflow::Smoothness)
}

/// "Buffer" row: a dropdown choosing which buffer the filled faces display.
fn view_row(ui: &mut egui::Ui, view: &mut BufferView, smoothness: bool) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::BUFFER)
            .describe(keys::ui_panels::BUFFER_DESCRIPTION)
            .page(PAGE),
        "buffer_view_combo",
        buffer_label(*view, smoothness),
        |ui| {
            for option in BufferView::ALL {
                ui.selectable_value(view, option, buffer_label(option, smoothness));
            }
        },
    );
}

/// The buffer's menu label, with the Roughness entry shown as "Smoothness" when the
/// Smoothness workflow is in effect (see [`shows_smoothness`]).
fn buffer_label(option: BufferView, smoothness: bool) -> review_localization::Key {
    let workflow = if smoothness {
        RoughnessWorkflow::Smoothness
    } else {
        RoughnessWorkflow::Roughness
    };
    labels::buffer_view_for(option, workflow)
}
