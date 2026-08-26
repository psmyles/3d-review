//! The Outliner: a two-tab list (a flat list of mesh objects + the deduplicated
//! materials) that drives the [`Selection`] used by the viewport highlight and the
//! Inspector.
//!
//! The Geometry tab is a *flat* list of the model's mesh-bearing nodes (the scene
//! graph's other node types aren't surfaced yet), each a stock visibility
//! checkbox + a selectable mesh name in default egui styling. The checkbox
//! hides/shows that mesh in the viewport
//! ([`UiState::hidden_meshes`]); clicking a row's name sets [`UiState::selection`]
//! (clicking the already-selected row clears it). Reads the scene nodes from the
//! borrowed [`ModelData`] (invariant 2: borrowed, not owned) and the material list
//! from the app→UI snapshot.

use review_model::{ModelData, SceneNode};
use review_render::Selection;

use crate::state::{OutlinerTab, UiState};
use crate::theme::size;
use crate::widgets;

/// The Outliner's tabs, in strip order. The index into this array is what
/// [`widgets::tab_bar`] hands back on a click.
const TABS: [OutlinerTab; 2] = [OutlinerTab::Geometry, OutlinerTab::Materials];

pub(crate) fn body(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    // ── Tabs: Geometry / Materials, as a full-width underlined tab strip ─────
    let materials_label = format!("Materials ({})", state.materials_snapshot.len());
    let labels = ["Geometry", materials_label.as_str()];
    let active = TABS
        .iter()
        .position(|tab| *tab == state.outliner_tab)
        .unwrap_or(0);
    if let Some(index) = widgets::tab_bar(ui, &labels, active) {
        state.outliner_tab = TABS[index];
    }
    ui.add_space(size::PANEL_ROW_GAP);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match state.outliner_tab {
            OutlinerTab::Geometry => geometry_tab(ui, state, model),
            OutlinerTab::Materials => materials_tab(ui, state),
        });
}

/// The Geometry tab: a flat list of the model's mesh-bearing nodes, each a stock
/// visibility checkbox + a selectable mesh name. Group / bone / other node types
/// are intentionally not listed (we don't act on them yet).
fn geometry_tab(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    let has_mesh = model.nodes.iter().any(|node| node.mesh_part.is_some());
    if !has_mesh {
        ui.weak("No meshes.");
        return;
    }

    let selection = state.selection;
    let mut clicked: Option<Selection> = None;

    for (index, node) in model.nodes.iter().enumerate() {
        if node.mesh_part.is_none() {
            continue;
        }
        let name = display_name(node, index);
        let selected = selection == Selection::Node(index);
        ui.horizontal(|ui| {
            // Visibility checkbox toggles membership in `hidden_meshes`.
            let mut visible = !state.hidden_meshes.contains(&index);
            if ui.checkbox(&mut visible, "").changed() {
                if visible {
                    state.hidden_meshes.remove(&index);
                } else {
                    state.hidden_meshes.insert(index);
                }
            }
            // Name toggles the selection.
            if ui.selectable_label(selected, &name).clicked() {
                clicked = Some(toggle(selected, Selection::Node(index)));
            }
        });
    }

    if let Some(new_selection) = clicked {
        state.selection = new_selection;
    }
}

/// The materials tab: a flat, deduplicated list of the model's editable materials,
/// each a stock selectable name row.
fn materials_tab(ui: &mut egui::Ui, state: &mut UiState) {
    if state.materials_snapshot.is_empty() {
        ui.weak("No materials.");
        return;
    }

    let selection = state.selection;
    let mut clicked: Option<Selection> = None;
    for (index, material) in state.materials_snapshot.iter().enumerate() {
        let selected = selection == Selection::Material(index);
        if ui.selectable_label(selected, &material.name).clicked() {
            clicked = Some(toggle(selected, Selection::Material(index)));
        }
    }

    if let Some(new_selection) = clicked {
        state.selection = new_selection;
    }
}

/// A node's display label: its source name, or a generated `Node N` fallback.
fn display_name(node: &SceneNode, index: usize) -> String {
    if node.name.is_empty() {
        format!("Node {index}")
    } else {
        node.name.clone()
    }
}

/// Clicking the already-selected row clears the selection; otherwise it selects
/// the clicked target.
fn toggle(already_selected: bool, target: Selection) -> Selection {
    if already_selected {
        Selection::None
    } else {
        target
    }
}
