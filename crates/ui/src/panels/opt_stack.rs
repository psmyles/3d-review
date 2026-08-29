//! The operation-stack pane, docked below the Outliner tree in the Opt
//! workspace.
//!
//! Layout, top to bottom: the preset row, the "Add operation" menu, the ordered
//! operation rows, and — once the stack has at least one operation — a pinned
//! "Export settings" row. Selecting any row (an operation or the export row)
//! puts its settings in the Inspector, so this pane stays a list and never grows
//! inline editors.
//!
//! Every mutation goes through [`UiState::opt`]'s `edit_stack`, which bumps the
//! revision `app` watches to schedule a reprocess.

use review_optimize::OpKind;

use crate::opt_state::{OptIntent, StackItem};
use crate::state::UiState;
use crate::theme::{color, size};
use crate::widgets;

/// Draw the pane. Returns the preset/export intent raised this frame, if any.
pub(crate) fn body(ui: &mut egui::Ui, state: &mut UiState) -> Option<OptIntent> {
    let mut intent = None;

    ui.add_space(size::PANEL_ROW_GAP);
    ui.horizontal(|ui| {
        if ui
            .button("Save preset")
            .on_hover_text("Write this operation stack to a JSON file")
            .clicked()
        {
            intent = Some(OptIntent::SavePreset);
        }
        if ui
            .button("Load preset")
            .on_hover_text("Replace this stack with one loaded from a JSON file")
            .clicked()
        {
            intent = Some(OptIntent::LoadPreset);
        }
    });

    ui.add_space(size::PANEL_ROW_GAP);
    add_menu(ui, state);
    ui.add_space(size::PANEL_ROW_GAP);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if state.opt.stack.ops.is_empty() {
                ui.add_space(size::PANEL_ROW_GAP);
                ui.label(
                    egui::RichText::new("No operations yet.\nAdd one above to start optimizing.")
                        .color(color::TEXT_MUTED),
                );
                return;
            }

            operation_rows(ui, state);

            ui.add_space(size::PANEL_ROW_GAP);
            ui.separator();
            export_row(ui, state);
        });

    intent
}

/// The "Add operation" dropdown. The LOD entry disables itself once the stack
/// has one: a second LOD operation would have to fan out an already-fanned-out
/// chain, which has no meaning.
fn add_menu(ui: &mut egui::Ui, state: &mut UiState) {
    let has_lod = state.opt.stack.has_lod();
    ui.menu_button("＋ Add operation", |ui| {
        widgets::style_combo_popup(ui);
        for make in OpKind::ALL {
            let kind = make();
            let is_lod = matches!(kind, OpKind::SimplifyLod(_));
            let enabled = !(is_lod && has_lod);
            let response = ui.add_enabled(enabled, egui::Button::new(kind.label()));
            let response = if enabled {
                response.on_hover_text(kind.description())
            } else {
                response.on_disabled_hover_text("A stack can hold only one LOD operation")
            };
            if response.clicked() {
                let id = state.opt.edit_stack_with(|stack| stack.push_op(kind));
                state.opt.selected = Some(StackItem::Op(id));
                ui.close();
            }
        }
    });
}

/// The ordered operation rows. Each carries an enable checkbox, the operation's
/// name (click to select), reorder arrows and a remove button.
fn operation_rows(ui: &mut egui::Ui, state: &mut UiState) {
    // Mutations are recorded and applied after the loop: the row widgets borrow
    // the stack to read it, and reordering mid-iteration would shift the indices
    // the remaining rows are drawn from.
    let mut toggled: Option<u64> = None;
    let mut moved: Option<(usize, isize)> = None;
    let mut removed: Option<u64> = None;
    let mut selected: Option<u64> = None;

    let count = state.opt.stack.ops.len();
    let button = size::OPT_STACK_ROW_BUTTON;

    for (index, op) in state.opt.stack.ops.iter().enumerate() {
        let is_selected = state.opt.selected == Some(StackItem::Op(op.id));
        ui.horizontal(|ui| {
            ui.set_height(size::OPT_STACK_ROW_HEIGHT);

            let mut enabled = op.enabled;
            if ui
                .checkbox(&mut enabled, "")
                .on_hover_text("Include this operation when processing")
                .changed()
            {
                toggled = Some(op.id);
            }

            // The reorder / remove buttons are laid out from the right edge so
            // the name gets whatever width is left and long labels truncate
            // rather than pushing the controls off the panel.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_sized([button, button], egui::Button::new("✕"))
                    .on_hover_text("Remove this operation")
                    .clicked()
                {
                    removed = Some(op.id);
                }
                if ui
                    .add_enabled_ui(index + 1 < count, |ui| {
                        ui.add_sized([button, button], egui::Button::new("▼"))
                    })
                    .inner
                    .on_hover_text("Move down (applied later)")
                    .clicked()
                {
                    moved = Some((index, 1));
                }
                if ui
                    .add_enabled_ui(index > 0, |ui| {
                        ui.add_sized([button, button], egui::Button::new("▲"))
                    })
                    .inner
                    .on_hover_text("Move up (applied earlier)")
                    .clicked()
                {
                    moved = Some((index, -1));
                }

                // A disabled operation is dimmed so a stack that is half switched
                // off reads at a glance.
                let text = egui::RichText::new(op.kind.label()).color(if !op.enabled {
                    color::TEXT_MUTED
                } else if is_selected {
                    color::TEXT_PRIMARY
                } else {
                    color::TEXT_BODY
                });
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(is_selected, text)
                        .on_hover_text(op.kind.description())
                        .clicked()
                    {
                        selected = Some(op.id);
                    }
                });
            });
        });
    }

    if let Some(id) = toggled {
        state.opt.edit_stack(|stack| {
            if let Some(op) = stack.op_mut(id) {
                op.enabled = !op.enabled;
            }
        });
    }
    if let Some((index, offset)) = moved {
        state.opt.edit_stack(|stack| {
            stack.reorder(index, offset);
        });
    }
    if let Some(id) = removed {
        state.opt.edit_stack(|stack| stack.remove_op(id));
        if state.opt.selected == Some(StackItem::Op(id)) {
            state.opt.selected = None;
        }
    }
    if let Some(id) = selected {
        state.opt.selected = Some(StackItem::Op(id));
    }
}

/// The pinned "Export settings" row. Not an operation — it neither reorders nor
/// removes — but it selects like one, putting the packaging / hierarchy / format
/// choices and the Export button in the Inspector.
fn export_row(ui: &mut egui::Ui, state: &mut UiState) {
    let is_selected = state.opt.selected == Some(StackItem::ExportSettings);
    ui.horizontal(|ui| {
        ui.set_height(size::OPT_STACK_ROW_HEIGHT);
        let text = egui::RichText::new("Export settings").color(if is_selected {
            color::TEXT_PRIMARY
        } else {
            color::TEXT_BODY
        });
        if ui
            .selectable_label(is_selected, text)
            .on_hover_text("Where and how the processed LOD chain is written")
            .clicked()
        {
            state.opt.selected = Some(StackItem::ExportSettings);
        }
    });
}
