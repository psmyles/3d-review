//! The operation-stack pane, docked below the Outliner tree in the Opt
//! workspace.
//!
//! Layout, top to bottom: the preset row, a divider, the "Add operation" menu,
//! the ordered operation rows, and — once the stack has at least one operation —
//! a pinned "Export settings" row. Selecting any row (an operation or the export
//! row) puts its settings in the Inspector, so this pane stays a list and never
//! grows inline editors.
//!
//! The rows are laid out like the Outliner's, by rect math rather than nested
//! layouts: the whole strip is the click target and the selection band spans it
//! edge to edge, which is what makes a list of rows read as one list.
//!
//! Every mutation goes through [`UiState::opt`]'s `edit_stack`, which bumps the
//! revision `app` watches to schedule a reprocess.

use review_optimize::{AoTarget, OpKind};
use review_render::{ActiveMaterial, VertexColorMode};

use crate::opt_state::{OptIntent, StackItem};
use crate::state::UiState;
use crate::theme::{color, size};
use crate::widgets::{self, wide_button};

/// Draw the pane. Returns the preset/export intent raised this frame, if any.
pub(crate) fn body(ui: &mut egui::Ui, state: &mut UiState) -> Option<OptIntent> {
    let mut intent = None;

    ui.add_space(size::PANEL_ROW_GAP);
    ui.horizontal(|ui| {
        // Two buttons splitting the pane's full width, so the pair reads as one
        // banded control rather than two tabs floating at the left edge.
        let gap = ui.spacing().item_spacing.x;
        let width = ((ui.available_width() - gap) * 0.5).max(0.0);
        if ui
            .add(wide_button("Save preset", width))
            .on_hover_text("Write this operation stack to a JSON file")
            .clicked()
        {
            intent = Some(OptIntent::SavePreset);
        }
        if ui
            .add(wide_button("Load preset", width))
            .on_hover_text("Replace this stack with one loaded from a JSON file")
            .clicked()
        {
            intent = Some(OptIntent::LoadPreset);
        }
    });

    ui.add_space(size::PANEL_ROW_GAP);
    ui.separator();
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
    let button = wide_button("+ Add operation", ui.available_width());
    // The full path: egui also carries a legacy top-level `menu` module, and it
    // is the one `egui::menu` resolves to.
    egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
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
                // A bake nobody can see helps nobody: adding one switches the
                // viewport to the Vertex Colors material, in the mode matching
                // the bake's write target. On add only — switching again on
                // select or edit would fight a user who deliberately went back
                // to Shaded.
                if let OpKind::BakeAo(params) = &kind {
                    state.debug.active_material = ActiveMaterial::VertexColors;
                    state.vertex_colors.mode = match params.target {
                        AoTarget::Alpha => VertexColorMode::Alpha,
                        _ => VertexColorMode::Rgb,
                    };
                }
                let id = state.opt.edit_stack_with(|stack| stack.push_op(kind));
                state.opt.selected = Some(StackItem::Op(id));
                ui.close();
            }
        }
    });
}

/// The ordered operation rows. Each carries an enable checkbox, the operation's
/// name (click anywhere on the row to select it), reorder arrows and a remove
/// button.
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
    let gap = ui.spacing().item_spacing.x;
    // Three trailing tiles (up / down / remove) and the gaps between them.
    let trailing = button * 3.0 + gap * 2.0;
    ui.spacing_mut().item_spacing.y = 0.0;

    for (index, op) in state.opt.stack.ops.iter().enumerate() {
        let is_selected = state.opt.selected == Some(StackItem::Op(op.id));
        let row_id = ui.id().with(("opt_stack_row", op.id));
        let (response, content, controls) = widgets::list_row(
            ui,
            row_id,
            is_selected,
            trailing,
            size::OPT_STACK_ROW_HEIGHT,
        );
        if response.clicked() {
            selected = Some(op.id);
        }
        response.on_hover_text(op.kind.description());

        // Enable checkbox, then the name filling whatever is left.
        let mut enabled = op.enabled;
        let check_rect =
            egui::Rect::from_min_size(content.left_top(), egui::vec2(button, content.height()));
        if ui
            .put(check_rect, egui::Checkbox::without_text(&mut enabled))
            .on_hover_text("Include this operation when processing")
            .changed()
        {
            toggled = Some(op.id);
        }

        // A disabled operation is dimmed so a stack that is half switched off
        // reads at a glance.
        let text = egui::RichText::new(op.kind.label()).color(if !op.enabled {
            color::TEXT_MUTED
        } else if is_selected {
            color::TEXT_PRIMARY
        } else {
            color::TEXT_BODY
        });
        let label_rect = egui::Rect::from_min_max(
            egui::pos2(check_rect.right() + gap, content.top()),
            content.max,
        );
        widgets::list_row_label(ui, label_rect, text);

        // Reorder / remove, laid out from the row's right edge.
        let tile = |slot: usize| {
            egui::Rect::from_min_size(
                egui::pos2(
                    controls.left() + slot as f32 * (button + gap),
                    controls.center().y - button * 0.5,
                ),
                egui::Vec2::splat(button),
            )
        };
        if ui
            .add_enabled_ui(index > 0, |ui| ui.put(tile(0), egui::Button::new("▲")))
            .inner
            .on_hover_text("Move up (applied earlier)")
            .clicked()
        {
            moved = Some((index, -1));
        }
        if ui
            .add_enabled_ui(index + 1 < count, |ui| {
                ui.put(tile(1), egui::Button::new("▼"))
            })
            .inner
            .on_hover_text("Move down (applied later)")
            .clicked()
        {
            moved = Some((index, 1));
        }
        if ui
            .put(tile(2), egui::Button::new("X"))
            .on_hover_text("Remove this operation")
            .clicked()
        {
            removed = Some(op.id);
        }
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
    let (response, content, _) = widgets::list_row(
        ui,
        ui.id().with("opt_export_row"),
        is_selected,
        0.0,
        size::OPT_STACK_ROW_HEIGHT,
    );
    if response.clicked() {
        state.opt.selected = Some(StackItem::ExportSettings);
    }
    response.on_hover_text("Where and how the processed mesh is written");

    let text = egui::RichText::new("Export settings").color(if is_selected {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_BODY
    });
    widgets::list_row_label(ui, content, text);
}
