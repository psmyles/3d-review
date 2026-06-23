//! The Inspector: edits the selected material's scalar/color parameters (emitting
//! the Phase-1 [`MaterialEdit`] intents) or shows read-only stats for a selected
//! node. Texture slots are stubbed here until Phase 3.
//!
//! Lays its rows out on the same striped two-column [`panel_grid`] the option
//! panels use (label column + control column, egui demo density) so a property
//! column reads consistently across the app.
//!
//! Reads the editable material values from the app→UI snapshot and the node
//! hierarchy from the borrowed [`ModelData`] (invariant 2). Returns the edit
//! intent (if any) for the overlay to forward to `app`.

use review_model::ModelData;
use review_render::{MaterialChange, MaterialEdit, Selection};

use crate::state::UiState;
use crate::widgets::{labeled_color_button, labeled_slider_with_value, panel_grid, value_row};

/// The seven PBR texture slots, shown stubbed until Phase 3 wires real textures.
const TEXTURE_SLOTS: [&str; 7] = [
    "Base Color",
    "Normal",
    "Roughness",
    "Metallic",
    "AO",
    "Emissive",
    "Opacity",
];

pub(crate) fn body(ui: &mut egui::Ui, state: &UiState, model: &ModelData) -> Option<MaterialEdit> {
    match state.selection {
        Selection::None => {
            ui.weak("Select a node or material in the Outliner.");
            None
        }
        Selection::Material(index) => material_inspector(ui, state, index),
        Selection::Node(index) => {
            node_inspector(ui, model, index);
            None
        }
    }
}

/// Editable material parameters for the selected slot — base color, metallic,
/// roughness, emissive — plus the stubbed texture slots, all on the shared
/// two-column table.
fn material_inspector(ui: &mut egui::Ui, state: &UiState, index: usize) -> Option<MaterialEdit> {
    let Some(snapshot) = state.materials_snapshot.get(index) else {
        ui.weak("Material no longer exists.");
        return None;
    };
    ui.heading(format!("Material — {}", snapshot.name));

    let current = snapshot.state;
    let mut edit = None;

    let mut base = [
        current.base_color.x,
        current.base_color.y,
        current.base_color.z,
    ];
    let mut metallic = current.metallic;
    let mut roughness = current.roughness;
    let mut emissive = [current.emissive.x, current.emissive.y, current.emissive.z];

    panel_grid(ui, "inspector_material", |ui| {
        if labeled_color_button(ui, "Base color", &mut base).changed() {
            edit = Some(MaterialEdit {
                index,
                change: MaterialChange::BaseColor(base),
            });
        }
        if labeled_slider_with_value(ui, "Metallic", &mut metallic, 0.0..=1.0, 3) {
            edit = Some(MaterialEdit {
                index,
                change: MaterialChange::Metallic(metallic),
            });
        }
        if labeled_slider_with_value(ui, "Roughness", &mut roughness, 0.0..=1.0, 3) {
            edit = Some(MaterialEdit {
                index,
                change: MaterialChange::Roughness(roughness),
            });
        }
        if labeled_color_button(ui, "Emissive", &mut emissive).changed() {
            edit = Some(MaterialEdit {
                index,
                change: MaterialChange::Emissive(emissive),
            });
        }
    });

    ui.separator();
    ui.heading("Texture slots");
    ui.weak("Assignable in Phase 3.");
    panel_grid(ui, "inspector_textures", |ui| {
        for slot in TEXTURE_SLOTS {
            // Stubbed slot: name in the label column, an em dash in the control
            // column (assignable in Phase 3).
            value_row(ui, slot, "\u{2014}");
        }
    });

    edit
}

/// Read-only stats for a selected node: name, type, child count, triangle count,
/// and world position (display metadata only — invariant 1).
fn node_inspector(ui: &mut egui::Ui, model: &ModelData, index: usize) {
    let Some(node) = model.nodes.get(index) else {
        ui.weak("Node no longer exists.");
        return;
    };

    let name = if node.name.is_empty() {
        format!("Node {index}")
    } else {
        node.name.clone()
    };
    ui.heading(format!("Node — {name}"));

    let kind = if node.mesh_part.is_some() {
        "Mesh part"
    } else {
        "Group"
    };
    let child_count = model
        .nodes
        .iter()
        .filter(|child| child.parent == Some(index))
        .count();
    // Own triangles (this node's mesh), not the whole subtree — a quick audit
    // figure that matches what selecting just this node would isolate.
    let triangle_count = model
        .tri_node
        .iter()
        .filter(|&&owner| owner as usize == index)
        .count();
    let position = node.transform.w_axis.truncate();

    panel_grid(ui, "inspector_node", |ui| {
        value_row(ui, "Type", kind);
        value_row(ui, "Children", &child_count.to_string());
        value_row(ui, "Triangles", &triangle_count.to_string());
        value_row(
            ui,
            "Position",
            &format!("{:.3}, {:.3}, {:.3}", position.x, position.y, position.z),
        );
    });
}
