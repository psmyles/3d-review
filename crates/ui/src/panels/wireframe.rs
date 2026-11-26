//! Wireframe tool options: line width, color, how collision meshes are drawn,
//! and a reset to defaults.

use super::aud_inspector::{committed_text_edit, parse_patterns};
use crate::docs::Page;
use crate::keys;
use crate::state::{
    ColliderDisplay, UiState, WireframePanelState,
    range::{WIREFRAME_WIDTH_MAX, WIREFRAME_WIDTH_MIN},
};
use crate::theme::color;
use crate::widgets::{
    Tip, color_swatch_row, grid_control, grid_label_tip, labeled_combo, labeled_slider_with_value,
    panel_footer, panel_grid,
};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsWireframe;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "wireframe", |ui| {
        labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_panels::WIREFRAME_WIDTH)
                .describe(keys::ui_panels::WIREFRAME_WIDTH_DESCRIPTION)
                .page(PAGE),
            &mut state.wireframe.width,
            WIREFRAME_WIDTH_MIN..=WIREFRAME_WIDTH_MAX,
            2,
        );
        color_swatch_row(
            ui,
            Tip::new(keys::ui_panels::WIREFRAME_COLOR)
                .describe(keys::ui_panels::WIREFRAME_COLOR_DESCRIPTION)
                .page(PAGE),
            &mut state.wireframe.color,
            &color::WIREFRAME_SWATCHES,
        );
    });
    ui.separator();
    // A grid of its own: a dropdown in the grid above, after its swatch row,
    // measured its column wider than the window's other panels.
    panel_grid(ui, "wireframe_colliders", |ui| {
        collider_rows(ui, &mut state.wireframe);
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.wireframe = WireframePanelState::default();
    }
}

/// How collision meshes are drawn, which names mark one, and their colour. Not
/// tied to the wireframe toggle: a collider drawn as edges shows whether the
/// wireframe is on or not.
fn collider_rows(ui: &mut egui::Ui, wireframe: &mut WireframePanelState) {
    let label = |display: ColliderDisplay| match display {
        ColliderDisplay::Wireframe => keys::ui_panels::COLLIDERS_WIREFRAME,
        ColliderDisplay::Shaded => keys::ui_panels::COLLIDERS_SHADED,
        ColliderDisplay::Hidden => keys::ui_panels::COLLIDERS_HIDDEN,
    };
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::COLLIDERS)
            .describe(keys::ui_panels::COLLIDERS_DESCRIPTION)
            .page(PAGE),
        "wireframe_colliders",
        label(wireframe.colliders),
        |ui| {
            crate::widgets::style_combo_popup(ui);
            for display in [
                ColliderDisplay::Wireframe,
                ColliderDisplay::Shaded,
                ColliderDisplay::Hidden,
            ] {
                ui.selectable_value(&mut wireframe.colliders, display, label(display));
            }
        },
    );

    grid_label_tip(
        ui,
        Tip::new(keys::ui_panels::COLLIDER_NAMES)
            .describe(keys::ui_panels::COLLIDER_NAMES_DESCRIPTION)
            .page(PAGE),
    );
    grid_control(ui, |ui| {
        let id = egui::Id::new("wireframe_collider_names");
        let mut valid = true;
        let (response, finished) =
            committed_text_edit(ui, id, &wireframe.collider_names.join(", "), |text| {
                valid = review_audit::glob::GlobSet::new(&parse_patterns(text)).is_ok();
                egui::TextEdit::singleline(text)
                    .hint_text(keys::ui_audit::PATTERN_HINT)
                    // The column's width, margins included: a set width is the
                    // text's alone, and would widen the window past its fellows.
                    .desired_width(f32::INFINITY)
                    .text_color(if valid {
                        color::TEXT_BODY
                    } else {
                        color::SEVERITY_ERROR
                    })
            });
        if !valid {
            let _ = response.on_hover_text(keys::ui_audit::PATTERN_INVALID);
        }
        if let Some(text) = finished {
            wireframe.collider_names = parse_patterns(&text);
        }
    });
    ui.end_row();

    color_swatch_row(
        ui,
        Tip::new(keys::ui_panels::COLLIDER_COLOR)
            .describe(keys::ui_panels::COLLIDER_COLOR_DESCRIPTION)
            .page(PAGE),
        &mut wireframe.collider_color,
        &color::COLLIDER_SWATCHES,
    );
}
