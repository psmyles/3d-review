//! The Inspector: edits the selected material's scalar/color parameters and its
//! seven texture slots (emitting the Phase-1/Phase-3 [`MaterialEdit`] intents and
//! the browse/clear intents `app` acts on), or shows read-only stats for a
//! selected node.
//!
//! The scalar editors lay out on the same striped two-column [`panel_grid`] the
//! option panels use; the texture slots use a compact per-slot block (name +
//! Browse/Clear + a channel dropdown for packed scalar maps).
//!
//! Reads the editable material values from the app→UI snapshot and the node
//! hierarchy from the borrowed [`ModelData`] (invariant 2). Returns the edit /
//! browse / clear intents (if any) for the overlay to forward to `app`.

use review_model::ModelData;
use review_render::{
    AlphaMode, ChannelSelect, MaterialChange, MaterialEdit, Selection, TextureSlot,
};

use crate::state::{TextureSlotRef, UiState};
use crate::widgets::{
    labeled_color_button, labeled_combo, labeled_slider_with_value, panel_grid, value_row,
};

/// What the Inspector emitted this frame, forwarded by the overlay into the
/// [`crate::state::UiOutput`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InspectorOutput {
    pub material_edit: Option<MaterialEdit>,
    pub browse: Option<TextureSlotRef>,
    pub clear: Option<TextureSlotRef>,
}

pub(crate) fn body(ui: &mut egui::Ui, state: &UiState, model: &ModelData) -> InspectorOutput {
    match state.selection {
        Selection::None => {
            ui.weak("Select a node or material in the Outliner.");
            InspectorOutput::default()
        }
        Selection::Material(index) => {
            egui::ScrollArea::vertical()
                .show(ui, |ui| material_inspector(ui, state, index))
                .inner
        }
        Selection::Node(index) => {
            node_inspector(ui, model, index);
            InspectorOutput::default()
        }
    }
}

/// Editable material parameters for the selected slot — base color, metallic,
/// roughness, emissive — plus the seven assignable texture slots and the alpha
/// compositing controls.
fn material_inspector(ui: &mut egui::Ui, state: &UiState, index: usize) -> InspectorOutput {
    let Some(snapshot) = state.materials_snapshot.get(index) else {
        ui.weak("Material no longer exists.");
        return InspectorOutput::default();
    };
    ui.heading(format!("Material — {}", snapshot.name));

    let current = &snapshot.state;
    let mut out = InspectorOutput::default();

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
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::BaseColor(base),
            });
        }
        if labeled_slider_with_value(ui, "Metallic", &mut metallic, 0.0..=1.0, 3) {
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::Metallic(metallic),
            });
        }
        if labeled_slider_with_value(ui, "Roughness", &mut roughness, 0.0..=1.0, 3) {
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::Roughness(roughness),
            });
        }
        if labeled_color_button(ui, "Emissive", &mut emissive).changed() {
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::Emissive(emissive),
            });
        }
    });

    ui.separator();
    ui.heading("Texture slots");
    for slot in TextureSlot::ALL {
        texture_slot_block(ui, current, index, slot, &mut out);
    }

    ui.separator();
    ui.heading("Transparency");
    alpha_controls(ui, current, index, &mut out);

    out
}

/// One texture slot's controls: the slot name, the assigned file (or "not
/// assigned"), a Browse / Clear pair, and — for an assigned packed scalar slot —
/// the channel dropdown (pre-filled by auto-detect, overridable here).
fn texture_slot_block(
    ui: &mut egui::Ui,
    state: &review_render::MaterialState,
    index: usize,
    slot: TextureSlot,
    out: &mut InspectorOutput,
) {
    let binding = state.textures[slot.index()].as_ref();
    let slot_ref = TextureSlotRef {
        material: index,
        slot: slot.index(),
    };

    ui.horizontal(|ui| {
        ui.strong(slot.label());
        // Right-aligned action buttons.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if binding.is_some() && ui.button("Clear").clicked() {
                out.clear = Some(slot_ref);
            }
            if ui.button("Browse\u{2026}").clicked() {
                out.browse = Some(slot_ref);
            }
        });
    });

    match binding {
        Some(binding) => {
            let name = binding
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("<texture>");
            ui.add(egui::Label::new(egui::RichText::new(name).weak()).truncate());
            // Packed scalar slots expose a channel selector; color/normal/emissive
            // always read RGB, so no selector is needed.
            if slot.is_scalar() {
                channel_dropdown(ui, index, slot, binding.channel, out);
            }
        }
        None => {
            ui.weak("not assigned");
        }
    }
    ui.add_space(ui.spacing().item_spacing.y);
}

/// The per-slot channel dropdown (R/G/B/A), emitting a re-route edit on change.
fn channel_dropdown(
    ui: &mut egui::Ui,
    index: usize,
    slot: TextureSlot,
    current: ChannelSelect,
    out: &mut InspectorOutput,
) {
    ui.horizontal(|ui| {
        ui.label("Channel");
        egui::ComboBox::from_id_salt(("inspector_channel", index, slot.index()))
            .selected_text(current.label())
            .show_ui(ui, |ui| {
                for choice in ChannelSelect::SCALAR {
                    if ui
                        .selectable_label(choice == current, choice.label())
                        .clicked()
                        && choice != current
                    {
                        out.material_edit = Some(MaterialEdit {
                            index,
                            change: MaterialChange::Channel(slot.index(), choice),
                        });
                    }
                }
            });
    });
}

/// Alpha compositing controls: the blend mode and (in Clip mode) the cutoff.
fn alpha_controls(
    ui: &mut egui::Ui,
    state: &review_render::MaterialState,
    index: usize,
    out: &mut InspectorOutput,
) {
    let mut cutoff = state.alpha_cutoff;
    panel_grid(ui, "inspector_alpha", |ui| {
        labeled_combo(
            ui,
            "Mode",
            "inspector_alpha_mode",
            state.alpha_mode.label(),
            |ui| {
                for mode in AlphaMode::ALL {
                    if ui
                        .selectable_label(mode == state.alpha_mode, mode.label())
                        .clicked()
                        && mode != state.alpha_mode
                    {
                        out.material_edit = Some(MaterialEdit {
                            index,
                            change: MaterialChange::AlphaMode(mode),
                        });
                    }
                }
            },
        );

        if state.alpha_mode == AlphaMode::Clip
            && labeled_slider_with_value(ui, "Cutoff", &mut cutoff, 0.0..=1.0, 3)
        {
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::AlphaCutoff(cutoff),
            });
        }
    });
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
