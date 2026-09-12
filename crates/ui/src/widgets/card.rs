//! The floating stats cards drawn over the viewport.

use crate::theme::{color, size};

/// A floating stats-style overlay card anchored bottom-left above the status
/// bar: the shared `Area` + framed card (translucent fill, hairline border,
/// fixed width) used by the model-stats and texture-stats panels so their look
/// can't drift apart.
pub(crate) fn stats_overlay_card(
    ctx: &egui::Context,
    id: &str,
    left_inset: f32,
    bottom_inset: f32,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    stats_overlay_card_at(
        ctx,
        id,
        StatsCardSide::Left,
        left_inset,
        bottom_inset,
        width,
        add_contents,
    );
}

/// Which viewport edge a stats card hugs. Opt shows two at once — the source
/// mesh's on the left as always, and the processed mesh's on the right — so they
/// read as the two halves of a comparison rather than a stack of cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatsCardSide {
    Left,
    Right,
    /// Centred in the viewport, for a card that describes the view as a whole
    /// rather than one mesh in it — the Opt overlay's legend. Its inset is the
    /// signed horizontal offset that keeps it centred between the side panels.
    Center,
}

/// [`stats_overlay_card`] with a choice of edge. `side_inset` is measured from
/// that edge (the width of the side panel docked there), so a card never lands
/// on top of a panel.
pub(crate) fn stats_overlay_card_at(
    ctx: &egui::Context,
    id: &str,
    side: StatsCardSide,
    side_inset: f32,
    bottom_inset: f32,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let margin = size::STATS_OVERLAY_MARGIN;
    let (align, offset_x) = match side {
        StatsCardSide::Left => (egui::Align2::LEFT_BOTTOM, side_inset + margin),
        StatsCardSide::Right => (egui::Align2::RIGHT_BOTTOM, -(side_inset + margin)),
        StatsCardSide::Center => (egui::Align2::CENTER_BOTTOM, side_inset),
    };
    egui::Area::new(egui::Id::new(id))
        .fade_in(false)
        .anchor(align, egui::vec2(offset_x, -(bottom_inset + margin)))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(color::STATS_OVERLAY_BG)
                .stroke(egui::Stroke::new(size::HAIRLINE, color::STATS_BORDER))
                .corner_radius(size::STATS_CORNER_RADIUS)
                .inner_margin(egui::Margin::symmetric(
                    size::STATS_PANEL_PAD_X,
                    size::STATS_PANEL_PAD_Y,
                ))
                .show(ui, |ui| {
                    ui.set_width(width);
                    add_contents(ui);
                });
        });
}
