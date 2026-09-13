//! The toolbar and status-bar bands: their group rects, and the toggles that
//! sit in them.
//!
//! These interiors are the last hand-laid (`scope_builder`) part of the chrome,
//! awaiting a native-layout rebuild; everything else is stock egui.

use crate::assets::AppIcon;
use crate::state::{OptionPanel, PanelsOpen};
use crate::theme::{color, size};
use crate::widgets::Tip;

use super::*;

/// The standard "toggle with an options panel" tile: left-click flips `flag`,
/// right-click toggles `panel` open. The triad (button + toggle + panel) repeats
/// across the toolbar and status bar; written once here.
pub(crate) fn option_toggle(
    ui: &mut egui::Ui,
    icon: &AppIcon,
    flag: &mut bool,
    panels_open: &mut PanelsOpen,
    panel: OptionPanel,
    tooltip: Tip,
) -> egui::Response {
    let response = icon_toggle_button_with_options(ui, icon, *flag, tooltip);
    if response.clicked() {
        *flag = !*flag;
    }
    if response.secondary_clicked() {
        panels_open.toggle(panel);
    }
    response
}

/// A chrome-bar group rect of `width`×`height`, vertically centered in
/// `bar_rect` and anchored `inset` in from the left (or, `from_right`, the
/// right) edge — the recessed toolbar/status-bar group placement.
pub(crate) fn bar_group_rect(
    bar_rect: egui::Rect,
    from_right: bool,
    inset: f32,
    width: f32,
    height: f32,
) -> egui::Rect {
    let x = if from_right {
        bar_rect.right() - inset - width
    } else {
        bar_rect.left() + inset
    };
    egui::Rect::from_min_size(
        egui::pos2(x, bar_rect.center().y - height * 0.5),
        egui::vec2(width, height),
    )
}

/// [`bar_group_rect`] for a group centred in the bar rather than mirrored to an
/// edge — for controls that describe the viewport as a whole rather than sitting
/// at one side of it.
pub(crate) fn bar_group_rect_centered(bar_rect: egui::Rect, width: f32, height: f32) -> egui::Rect {
    egui::Rect::from_center_size(bar_rect.center(), egui::vec2(width, height))
}

/// Lay `add_contents` out left-to-right inside `rect` at the standard group
/// height — the scope every chrome-bar group opens.
pub(crate) fn bar_group_scope(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    group_height: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.set_height(group_height);
            add_contents(ui);
        },
    );
}

/// A recessed group shell that hosts a row of toolbar tiles, laid out
/// left-to-right with the standard inter-icon gap.
pub(crate) fn toolbar_group_shell(
    ui: &mut egui::Ui,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let group_height = size::TOOLBAR_GROUP_HEIGHT;
    let desired = egui::vec2(width, group_height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
    ui.painter().rect(
        rect,
        size::TILE_CORNER_RADIUS,
        color::GROUP_BG,
        egui::Stroke::NONE,
        egui::StrokeKind::Outside,
    );
    let inner_padding = size::TOOLBAR_GROUP_PADDING;
    let inner = rect.shrink2(egui::vec2(inner_padding, inner_padding));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = size::TOOLBAR_ICON_GAP;
            add_contents(ui);
        },
    );
    response
}
