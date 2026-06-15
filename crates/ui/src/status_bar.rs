//! The bottom status bar: currently just the Model Stats toggle button, centered
//! and inset equally from every edge.

use crate::assets::ICON_INFO;
use crate::state::UiState;
use crate::theme::{self, color, size};
use crate::widgets::{icon_toggle_button, toolbar_group_shell};

/// Background frame for the status bar: matches the toolbar fill with a top
/// border. Zero inner margin — content is placed by rect math in [`draw`].
fn status_bar_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(color::CHROME_BG)
        .stroke(egui::Stroke::new(size::HAIRLINE, color::STATUS_BORDER))
        .inner_margin(egui::Margin::same(0))
}

pub(crate) fn draw(ctx: &egui::Context, state: &mut UiState) {
    let status_bar_height = theme::px(ctx, size::STATUS_BAR_HEIGHT);
    let group_height = theme::px(ctx, size::TOOLBAR_GROUP_HEIGHT);
    let single_icon_group_width = theme::px(ctx, size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH);

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(status_bar_height)
        .frame(status_bar_frame())
        .show(ctx, |ui| {
            // Inset the group equally on all sides: the vertical gap is fixed by
            // centering the group in the bar, so use that same gap on the left
            // edge to keep the button box equidistant from every edge.
            let bar_rect = ui.max_rect();
            let edge_inset = (status_bar_height - group_height) * 0.5;
            let group_rect = egui::Rect::from_min_size(
                egui::pos2(
                    bar_rect.left() + edge_inset,
                    bar_rect.center().y - group_height * 0.5,
                ),
                egui::vec2(single_icon_group_width, group_height),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(group_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_height(group_height);
                    toolbar_group_shell(ui, ctx, single_icon_group_width, |ui| {
                        if icon_toggle_button(ui, ctx, &ICON_INFO, state.show_stats, "Model Stats")
                            .clicked()
                        {
                            state.show_stats = !state.show_stats;
                        }
                    });
                },
            );
        });
}
