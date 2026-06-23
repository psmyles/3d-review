//! The bottom status bar: the Model Stats toggle inset on the left, and a
//! rendering-quality group (IBL / Bloom / SSAO / Anti aliasing) mirrored to the
//! right.

use crate::assets::{
    ICON_ANTI_ALIASING, ICON_AO, ICON_BLOOM, ICON_IBL, ICON_INFO, ICON_TONEMAPPER,
};
use crate::state::{OptionPanel, UiState, WorkspaceMode};
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
    let quint_icon_group_width = theme::px(ctx, size::TOOLBAR_QUINT_ICON_GROUP_WIDTH);

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(status_bar_height)
        .frame(status_bar_frame())
        .show(ctx, |ui| {
            // The Model Stats overlay only renders in the 3D workspace, so its
            // toggle is dead weight elsewhere — drop the button in UV / Texture
            // mode and leave a clean status bar.
            if state.mode != WorkspaceMode::ThreeD {
                return;
            }

            // Inset the group equally on all sides: the vertical gap is fixed by
            // centering the group in the bar, so use that same gap on the left
            // edge to keep the button box equidistant from every edge.
            let bar_rect = ui.max_rect();
            let edge_inset = (status_bar_height - group_height) * 0.5;

            // Model Stats toggle — single-icon group inset equally on the left.
            let left_rect = egui::Rect::from_min_size(
                egui::pos2(
                    bar_rect.left() + edge_inset,
                    bar_rect.center().y - group_height * 0.5,
                ),
                egui::vec2(single_icon_group_width, group_height),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
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

            // Rendering-quality group — IBL / Bloom / SSAO / Tonemapper / Anti
            // aliasing in one recessed group mirrored to the right edge. Each tile
            // left-clicks to toggle its effect (highlighted while on) and
            // right-clicks to open its options panel, matching the top-toolbar
            // buttons.
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(
                    bar_rect.right() - edge_inset - quint_icon_group_width,
                    bar_rect.center().y - group_height * 0.5,
                ),
                egui::vec2(quint_icon_group_width, group_height),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_height(group_height);
                    toolbar_group_shell(ui, ctx, quint_icon_group_width, |ui| {
                        // Image-based lighting. Disabled + forced off when the
                        // adapter can't build the IBL maps (invariant 4).
                        if !state.ibl_supported {
                            state.environment.ibl_enabled = false;
                        }
                        ui.add_enabled_ui(state.ibl_supported, |ui| {
                            let ibl = icon_toggle_button(
                                ui,
                                ctx,
                                &ICON_IBL,
                                state.environment.ibl_enabled,
                                "Image-based lighting (right-click for options)",
                            );
                            if ibl.clicked() {
                                state.environment.ibl_enabled = !state.environment.ibl_enabled;
                            }
                            if ibl.secondary_clicked() {
                                state.panels_open.toggle(OptionPanel::Environment);
                            }
                        });

                        // Bloom (HDR glow).
                        let bloom = icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_BLOOM,
                            state.bloom.enabled,
                            "Bloom (right-click for options)",
                        );
                        if bloom.clicked() {
                            state.bloom.enabled = !state.bloom.enabled;
                        }
                        if bloom.secondary_clicked() {
                            state.panels_open.toggle(OptionPanel::Bloom);
                        }

                        // Screen-space ambient occlusion. Disabled + forced off when
                        // the adapter can't run SSAO (invariant 4).
                        if !state.ssao_supported {
                            state.ssao.enabled = false;
                        }
                        ui.add_enabled_ui(state.ssao_supported, |ui| {
                            let ssao = icon_toggle_button(
                                ui,
                                ctx,
                                &ICON_AO,
                                state.ssao.enabled,
                                "Ambient occlusion (right-click for options)",
                            );
                            if ssao.clicked() {
                                state.ssao.enabled = !state.ssao.enabled;
                            }
                            if ssao.secondary_clicked() {
                                state.panels_open.toggle(OptionPanel::Ssao);
                            }
                        });

                        // Tone mapping. Left-click toggles the tone curve on/off
                        // (off = linear → sRGB); right-click opens the operator
                        // picker.
                        let tonemap = icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_TONEMAPPER,
                            state.tonemap.enabled,
                            "Tone mapping (right-click for options)",
                        );
                        if tonemap.clicked() {
                            state.tonemap.enabled = !state.tonemap.enabled;
                        }
                        if tonemap.secondary_clicked() {
                            state.panels_open.toggle(OptionPanel::Tonemap);
                        }

                        // Anti aliasing.
                        let aa = icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_ANTI_ALIASING,
                            state.anti_aliasing.enabled,
                            "Anti aliasing (right-click for options)",
                        );
                        if aa.clicked() {
                            state.anti_aliasing.enabled = !state.anti_aliasing.enabled;
                        }
                        if aa.secondary_clicked() {
                            state.panels_open.toggle(OptionPanel::AntiAliasing);
                        }
                    });
                },
            );
        });
}
