//! The bottom status bar: the Model Stats toggle inset on the left, and a
//! rendering-quality group (IBL / AO / Tonemapper / Anti aliasing) mirrored to the
//! right.

use crate::assets::{ICON_ANTI_ALIASING, ICON_AO, ICON_IBL, ICON_INFO, ICON_TONEMAPPER};
use crate::state::{OptionPanel, TexViewRequest, TextureBackground, UiState, WorkspaceMode};
use crate::theme::{self, color, font, size};
use crate::widgets::{
    icon_toggle_button, icon_toggle_button_with_options, segment_button, toolbar_group_shell,
};

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
    let quad_icon_group_width = theme::px(ctx, size::TOOLBAR_QUAD_ICON_GROUP_WIDTH);

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(status_bar_height)
        .frame(status_bar_frame())
        .show(ctx, |ui| {
            // The UV workspace keeps a clean status bar (its tools are in the
            // toolbar). The 3D workspace shows the model-stats toggle + the
            // rendering-quality group; the Texture workspace shows the texture-stats
            // toggle + the background-fill group.
            match state.mode {
                WorkspaceMode::Uv => return,
                WorkspaceMode::ThreeD => {}
                WorkspaceMode::Texture => {
                    draw_texture_status_bar(ui, ctx, state, group_height);
                    return;
                }
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

            // Rendering-quality group — IBL / AO / Tonemapper / Anti aliasing in
            // one recessed group mirrored to the right edge. Each tile
            // left-clicks to toggle its effect (highlighted while on) and
            // right-clicks to open its options panel, matching the top-toolbar
            // buttons.
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(
                    bar_rect.right() - edge_inset - quad_icon_group_width,
                    bar_rect.center().y - group_height * 0.5,
                ),
                egui::vec2(quad_icon_group_width, group_height),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_height(group_height);
                    toolbar_group_shell(ui, ctx, quad_icon_group_width, |ui| {
                        // Image-based lighting. Disabled + forced off when the
                        // adapter can't build the IBL maps (invariant 4).
                        if !state.ibl_supported {
                            state.environment.ibl_enabled = false;
                        }
                        ui.add_enabled_ui(state.ibl_supported, |ui| {
                            let ibl = icon_toggle_button_with_options(
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

                        // Ambient occlusion (GTAO). Disabled + forced off when the
                        // adapter can't run it (invariant 4).
                        if !state.gtao_supported {
                            state.gtao.enabled = false;
                        }
                        ui.add_enabled_ui(state.gtao_supported, |ui| {
                            let gtao = icon_toggle_button_with_options(
                                ui,
                                ctx,
                                &ICON_AO,
                                state.gtao.enabled,
                                "Ambient occlusion (right-click for options)",
                            );
                            if gtao.clicked() {
                                state.gtao.enabled = !state.gtao.enabled;
                            }
                            if gtao.secondary_clicked() {
                                state.panels_open.toggle(OptionPanel::Gtao);
                            }
                        });

                        // Tone mapping. Left-click toggles the tone curve on/off
                        // (off = linear → sRGB); right-click opens the operator
                        // picker.
                        let tonemap = icon_toggle_button_with_options(
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
                        let aa = icon_toggle_button_with_options(
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

/// The Texture-workspace status bar: the texture-stats toggle inset on the left
/// (the Tex viewport's analogue of the model-stats info button) and the
/// background-fill radio group (Black / White / Grey / Checker) mirrored to the
/// right. Laid out with the same edge inset as the 3D bar.
fn draw_texture_status_bar(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    state: &mut UiState,
    group_height: f32,
) {
    let single_icon_group_width = theme::px(ctx, size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH);
    let bg_group_width = theme::px(ctx, size::TEXTURE_BG_GROUP_WIDTH);
    let segment_w = theme::px(ctx, size::TEXTURE_BG_SEGMENT_WIDTH);

    let bar_rect = ui.max_rect();
    let edge_inset = (bar_rect.height() - group_height) * 0.5;

    // Texture-stats toggle — single-icon group inset equally on the left.
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
                if icon_toggle_button(
                    ui,
                    ctx,
                    &ICON_INFO,
                    state.texture_view.show_stats,
                    "Texture Info",
                )
                .clicked()
                {
                    state.texture_view.show_stats = !state.texture_view.show_stats;
                }
            });
        },
    );

    // Zoom-level readout next to the texture-info button: the current zoom as an
    // integer percentage, clickable to reset to 100%.
    let zoom_label_width = theme::px(ctx, size::TEXTURE_ZOOM_LABEL_WIDTH);
    let zoom_rect = egui::Rect::from_min_size(
        egui::pos2(
            left_rect.right() + edge_inset,
            bar_rect.center().y - group_height * 0.5,
        ),
        egui::vec2(zoom_label_width, group_height),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(zoom_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.set_height(group_height);
            if zoom_reset_label(ui, ctx, state.texture_view.zoom).clicked() {
                // Toggle on the readout the user sees: at 100% → fit, at any other
                // zoom → 100%. Both are emitted as a request `texture_view` eases
                // to over `TEXTURE_ZOOM_ANIM_SECS` on its next paint.
                state.texture_view.request =
                    Some(if (state.texture_view.zoom * 100.0).round() as i32 == 100 {
                        TexViewRequest::Fit
                    } else {
                        TexViewRequest::Zoom(1.0)
                    });
            }
        },
    );

    // Background-fill radio group — mirrored to the right edge.
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(
            bar_rect.right() - edge_inset - bg_group_width,
            bar_rect.center().y - group_height * 0.5,
        ),
        egui::vec2(bg_group_width, group_height),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(right_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.set_height(group_height);
            toolbar_group_shell(ui, ctx, bg_group_width, |ui| {
                for background in TextureBackground::ALL {
                    let selected = state.texture_view.background == background;
                    let response = segment_button(ui, ctx, background.label(), selected, segment_w)
                        .on_hover_text(background_tooltip(background));
                    if response.clicked() {
                        state.texture_view.background = background;
                    }
                }
            });
        },
    );
}

/// A clickable zoom-percentage readout for the Tex status bar: shows the current
/// zoom rounded to the nearest integer percent (e.g. `120%`), brightening on
/// hover; clicking it toggles between 100% and fit-to-view.
fn zoom_reset_label(ui: &mut egui::Ui, ctx: &egui::Context, zoom: f32) -> egui::Response {
    let percent = (zoom * 100.0).round() as i32;
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
    let text_color = if response.hovered() {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_MUTED
    };
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        format!("{percent}%"),
        egui::FontId::monospace(theme::px(ctx, font::STATUS_ZOOM)),
        text_color,
    );
    response.on_hover_text("Zoom level - click to toggle 100% / fit to view")
}

/// Hover tooltip spelling out a background-fill segment's single-letter label.
fn background_tooltip(background: TextureBackground) -> &'static str {
    match background {
        TextureBackground::Black => "Black background",
        TextureBackground::White => "White background",
        TextureBackground::Grey => "Grey background",
        TextureBackground::Checker => "Checker background",
    }
}
