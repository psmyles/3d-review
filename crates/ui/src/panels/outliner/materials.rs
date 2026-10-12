//! The Outliner's Materials tab: the deduplicated material list, narrowed by the
//! header's search box and driving the same [`Selection`] the Scene tab does.

use review_render::Selection;

use super::{matches_search, toggle};
use crate::keys;
use crate::state::UiState;
use crate::theme::{color, size};
use crate::widgets;

/// The materials tab: a flat, deduplicated list of the model's editable materials,
/// each a full-width [`widgets::list_row`] like the other tabs, narrowed by the header's search box. Indices
/// stay the snapshot's own, so filtering never re-points a [`Selection::Material`].
pub(super) fn materials_tab(ui: &mut egui::Ui, state: &mut UiState) {
    if state.materials_snapshot.is_empty() {
        ui.weak(keys::ui_outliner::NO_MATERIALS);
        return;
    }

    let query = state.outliner.search.trim().to_lowercase();
    let selection = state.selection;
    let mut clicked: Option<Selection> = None;
    let mut matched = false;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (index, material) in state.materials_snapshot.iter().enumerate() {
        if !matches_search(&material.name, &query) {
            continue;
        }
        matched = true;
        let selected = selection == Selection::Material(index);
        let (response, content, _) = widgets::list_row(
            ui,
            ui.id().with(("material", index)),
            selected,
            0.0,
            size::OUTLINER_ROW_HEIGHT,
        );
        let name_color = if selected {
            color::TEXT_PRIMARY
        } else {
            color::TEXT_BODY
        };
        widgets::list_row_label(
            ui,
            content,
            egui::RichText::new(&material.name).color(name_color),
        );
        if response.clicked() {
            clicked = Some(toggle(selected, Selection::Material(index)));
        }
    }
    if !matched {
        ui.weak(keys::ui_outliner::NO_MATCHES);
    }

    if let Some(new_selection) = clicked {
        state.select_material(new_selection);
    }
}
