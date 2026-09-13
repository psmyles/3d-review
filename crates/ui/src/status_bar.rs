//! The bottom status bar: the Model Stats toggle inset on the left, a
//! rendering-quality group (IBL / AO / Tonemapper / Anti aliasing) mirrored to the
//! right, and the centre span between them — which carries the Opt workspace's
//! comparison controls, over the split's divider, or in the 3D workspace the
//! animation transport while a clip is selected. The two never want the slot at
//! once: the transport doesn't draw in Opt, and the comparison controls exist
//! only there.

use crate::assets::{
    ICON_ANTI_ALIASING, ICON_AO, ICON_BACKGROUND, ICON_IBL, ICON_INFO, ICON_OPT_OVERLAY,
    ICON_OPT_SPLIT, ICON_OPT_SWAP, ICON_OPT_SYNC, ICON_TONEMAPPER,
};
use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::opt_state::OptLayout;
use crate::state::{OptionPanel, TexViewRequest, TextureBackground, UiState, WorkspaceMode};
use crate::theme::{color, font, size};
use crate::transport;
use crate::widgets::{
    Tip, bar_group_rect, bar_group_rect_centered, bar_group_scope, compact_combo,
    icon_toggle_button, icon_toggle_button_with_options, option_toggle, segment_button, tip,
    toolbar_group_shell,
};
use review_model::ModelData;
use review_render::ViewportBackground;

/// Background frame for the status bar: matches the toolbar fill with a top
/// border. Zero inner margin — content is placed by rect math in [`draw`].
fn status_bar_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(color::CHROME_BG)
        .stroke(egui::Stroke::new(size::HAIRLINE, color::STATUS_BORDER))
        .inner_margin(egui::Margin::same(0))
}

pub(crate) fn draw(root: &mut egui::Ui, state: &mut UiState, model: &ModelData) {
    let status_bar_height = size::STATUS_BAR_HEIGHT;
    let group_height = size::TOOLBAR_GROUP_HEIGHT;
    let single_icon_group_width = size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH;
    let quint_icon_group_width = size::TOOLBAR_QUINT_ICON_GROUP_WIDTH;

    egui::Panel::bottom("status_bar")
        .exact_size(status_bar_height)
        .frame(status_bar_frame())
        .show(root, |ui| {
            // The UV workspace keeps a clean status bar (its tools are in the
            // toolbar). The 3D workspace shows the model-stats toggle + the
            // rendering-quality group; the Texture workspace shows the texture-stats
            // toggle + the background-fill group.
            match state.mode {
                WorkspaceMode::Uv => return,
                // Opt draws the same 3D scene through the same shading and
                // rendering-quality controls, so it shares this bar unchanged.
                WorkspaceMode::ThreeD | WorkspaceMode::Opt => {}
                WorkspaceMode::Texture => {
                    draw_texture_status_bar(ui, state, group_height);
                    return;
                }
            }

            // Inset the group equally on all sides: the vertical gap is fixed by
            // centering the group in the bar, so use that same gap on the left
            // edge to keep the button box equidistant from every edge.
            let bar_rect = ui.max_rect();
            let edge_inset = (status_bar_height - group_height) * 0.5;

            // Model Stats toggle — single-icon group inset equally on the left.
            let left_rect = bar_group_rect(
                bar_rect,
                false,
                edge_inset,
                single_icon_group_width,
                group_height,
            );
            bar_group_scope(ui, left_rect, group_height, |ui| {
                toolbar_group_shell(ui, single_icon_group_width, |ui| {
                    if icon_toggle_button(
                        ui,
                        &ICON_INFO,
                        state.show_stats,
                        Tip::new(keys::ui_status_bar::MODEL_STATS)
                            .describe(keys::ui_status_bar::MODEL_STATS_DESCRIPTION)
                            .page(Page::Stats),
                    )
                    .clicked()
                    {
                        state.show_stats = !state.show_stats;
                    }
                });
            });

            // Rendering-quality group — Background / IBL / AO / Tonemapper / Anti
            // aliasing in one recessed group mirrored to the right edge. Each tile
            // left-clicks (cycle / toggle, highlighted while non-default / on) and
            // right-clicks to open its options panel, matching the top-toolbar
            // buttons.
            let right_rect = bar_group_rect(
                bar_rect,
                true,
                edge_inset,
                quint_icon_group_width,
                group_height,
            );
            bar_group_scope(ui, right_rect, group_height, |ui| {
                toolbar_group_shell(ui, quint_icon_group_width, |ui| {
                    // Viewport background. Left-click cycles the presets (not a
                    // plain toggle, so it stays hand-wired), right-click opens the
                    // Background options panel; highlighted while a non-default
                    // (non-black) background is active.
                    let background = icon_toggle_button_with_options(
                        ui,
                        &ICON_BACKGROUND,
                        state.viewport_background != ViewportBackground::Black,
                        Tip::new(keys::ui_status_bar::VIEWPORT_BACKGROUND)
                            .describe(keys::ui_status_bar::VIEWPORT_BACKGROUND_DESCRIPTION)
                            .with_options()
                            .page(Page::PanelsBackground),
                    );
                    if background.clicked() {
                        state.viewport_background = state.viewport_background.next();
                    }
                    if background.secondary_clicked() {
                        state.panels_open.toggle(OptionPanel::Background);
                    }

                    // Image-based lighting. Disabled + forced off when the
                    // adapter can't build the IBL maps (invariant 4).
                    if !state.capabilities.ibl {
                        state.environment.ibl_enabled = false;
                    }
                    ui.add_enabled_ui(state.capabilities.ibl, |ui| {
                        option_toggle(
                            ui,
                            &ICON_IBL,
                            &mut state.environment.ibl_enabled,
                            &mut state.panels_open,
                            OptionPanel::Environment,
                            Tip::new(keys::ui_status_bar::IBL)
                                .describe(keys::ui_status_bar::IBL_DESCRIPTION)
                                .with_options()
                                .page(Page::PanelsEnvironment),
                        );
                    });

                    // Ambient occlusion (GTAO). Disabled + forced off when the
                    // adapter can't run it (invariant 4).
                    if !state.capabilities.gtao {
                        state.gtao.enabled = false;
                    }
                    ui.add_enabled_ui(state.capabilities.gtao, |ui| {
                        option_toggle(
                            ui,
                            &ICON_AO,
                            &mut state.gtao.enabled,
                            &mut state.panels_open,
                            OptionPanel::Gtao,
                            Tip::new(keys::ui_status_bar::AO)
                                .describe(keys::ui_status_bar::AO_DESCRIPTION)
                                .with_options()
                                .page(Page::PanelsAmbientOcclusion),
                        );
                    });

                    // Tone mapping (off = linear → sRGB), then anti-aliasing.
                    option_toggle(
                        ui,
                        &ICON_TONEMAPPER,
                        &mut state.tonemap.enabled,
                        &mut state.panels_open,
                        OptionPanel::Tonemap,
                        Tip::new(keys::ui_status_bar::TONEMAP)
                            .describe(keys::ui_status_bar::TONEMAP_DESCRIPTION)
                            .with_options()
                            .page(Page::PanelsTonemapper),
                    );
                    option_toggle(
                        ui,
                        &ICON_ANTI_ALIASING,
                        &mut state.anti_aliasing.enabled,
                        &mut state.panels_open,
                        OptionPanel::AntiAliasing,
                        Tip::new(keys::ui_status_bar::ANTI_ALIASING)
                            .describe(keys::ui_status_bar::ANTI_ALIASING_DESCRIPTION)
                            .with_options()
                            .page(Page::PanelsAntiAliasing),
                    );
                });
            });

            // The centre span between the two mirrored groups: Opt's comparison
            // controls, or the 3D workspace's animation transport.
            if state.mode == WorkspaceMode::Opt {
                draw_opt_group(ui, state, bar_rect, group_height);
            } else {
                draw_transport(ui, state, model, left_rect, right_rect, group_height);
            }
        });
}

/// The animation transport, centred in the free span between the bar's two
/// mirrored groups — drawn only while a clip is selected.
///
/// Centred in that **span**, not in the bar: the left group is one icon wide and
/// the right one is five, so the bar's own centre sits left of the gap's and a
/// full-width transport would reach the quality group before it reached the
/// stats toggle. The span is also what caps the width, so the row degrades into
/// the room it has (see [`transport::transport_row`]) instead of running under
/// its neighbours the way the old floating card did.
fn draw_transport(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &ModelData,
    left_rect: egui::Rect,
    right_rect: egui::Rect,
    group_height: f32,
) {
    let Some(clip) = state
        .animation
        .selected_clip
        .and_then(|index| model.animations.get(index))
    else {
        return;
    };
    let spacing = size::TOOLBAR_GROUP_SPACING;
    let span = right_rect.left() - left_rect.right() - spacing * 2.0;
    let width = span.min(size::ANIM_TRANSPORT_WIDTH);
    // A window narrow enough to leave no gap at all: the two icon groups are the
    // controls that must survive, so the transport simply doesn't draw.
    if width < size::ANIM_TRANSPORT_MIN_WIDTH {
        return;
    }
    let centre = egui::pos2(
        (left_rect.right() + right_rect.left()) * 0.5,
        left_rect.center().y,
    );
    let rect = egui::Rect::from_center_size(centre, egui::vec2(width, group_height));
    let fps = model.frame_rate_or_default();
    bar_group_scope(ui, rect, group_height, |ui| {
        transport::transport_row(ui, state, clip, fps);
    });
}

/// The Opt comparison controls, centred in the status bar: which LOD level is
/// displayed, how the two meshes are laid out, whether the split's views share a
/// camera, and which mesh reads as solid.
///
/// Centred rather than mirrored to an edge like the bar's other groups — these
/// describe what the viewport above is showing, and the split's divider is the
/// centre, so that is where they belong.
///
/// Three tiles: the two layouts, then whichever of camera-sync / A-B-swap the
/// active layout can act on. Those two are mutually exclusive — only the split
/// has two cameras to link, only the overlay draws one mesh over another — so
/// showing both would always leave one inert. The group's width is the same
/// either way, so swapping the third tile shifts nothing.
fn draw_opt_group(ui: &mut egui::Ui, state: &mut UiState, bar_rect: egui::Rect, group_height: f32) {
    let has_result = state.opt.has_result();
    let level_count = state.opt.level_count();
    let picker_width = size::TOOLBAR_OPT_LOD_DROPDOWN_WIDTH;
    let icons_width = size::TOOLBAR_TRIPLE_ICON_GROUP_WIDTH;
    let spacing = size::TOOLBAR_GROUP_SPACING;

    // The picker appears only once the chain actually has levels to choose
    // between, so the centred width has to account for it either way.
    let show_picker = has_result && level_count > 1;
    let width = if show_picker {
        picker_width + spacing + icons_width
    } else {
        icons_width
    };
    let rect = bar_group_rect_centered(bar_rect, width, group_height);
    bar_group_scope(ui, rect, group_height, |ui| {
        ui.spacing_mut().item_spacing.x = spacing;

        if show_picker {
            state.opt.active_lod = state.opt.active_lod.min(level_count - 1);
            let before = state.opt.active_lod;
            let selected = lod_label(state.opt.active_lod);
            compact_combo(ui, "opt_lod_picker", picker_width, selected, |ui| {
                for level in 0..level_count {
                    ui.selectable_value(&mut state.opt.active_lod, level, lod_label(level));
                }
            });
            // Once the user has said which level they want, later runs keep showing
            // it instead of jumping back to the one the workspace chose for them.
            if state.opt.active_lod != before {
                state.opt.lod_pinned = true;
            }
        }

        let split = state.opt.layout == OptLayout::Split;
        toolbar_group_shell(ui, icons_width, |ui| {
            if icon_toggle_button(
                ui,
                &ICON_OPT_SPLIT,
                split,
                Tip::new(keys::ui_status_bar::SPLIT_VIEW)
                    .describe(keys::ui_status_bar::SPLIT_VIEW_DESCRIPTION)
                    .page(Page::OptComparison),
            )
            .clicked()
            {
                state.opt.layout = OptLayout::Split;
            }
            if icon_toggle_button(
                ui,
                &ICON_OPT_OVERLAY,
                !split,
                Tip::new(keys::ui_status_bar::OVERLAY_VIEW)
                    .describe(keys::ui_status_bar::OVERLAY_VIEW_DESCRIPTION)
                    .page(Page::OptComparison),
            )
            .clicked()
            {
                state.opt.layout = OptLayout::Overlay;
            }

            if split {
                // Camera sync: only the split has a second camera to link.
                let sync = state.opt.camera_sync;
                if icon_toggle_button(
                    ui,
                    &ICON_OPT_SYNC,
                    sync,
                    Tip::new(keys::ui_status_bar::SYNC_CAMERAS).page(Page::OptComparison),
                )
                .clicked()
                {
                    state.opt.camera_sync = !sync;
                }
            } else {
                // The A/B swap: which mesh reads as solid. Disabled until there
                // is something to swap to, so the button never lies about what
                // the viewport is showing.
                let response = ui
                    .add_enabled_ui(has_result, |ui| {
                        icon_toggle_button(
                            ui,
                            &ICON_OPT_SWAP,
                            false,
                            Tip::new(keys::ui_status_bar::swap_sides(
                                review_localization::tr(labels::comparison_side(state.opt.side))
                                    .into_owned(),
                            ))
                            .describe(keys::ui_status_bar::SWAP_SIDES_DESCRIPTION)
                            .page(Page::OptComparison),
                        )
                    })
                    .inner;
                if response.clicked() {
                    state.opt.side = state.opt.side.swapped();
                }
                tip(
                    response,
                    Tip::new(keys::ui_status_bar::NOTHING_PROCESSED)
                        .describe(keys::ui_status_bar::NOTHING_PROCESSED_DESCRIPTION)
                        .page(Page::OptRun),
                );
            }
        });
    });
}

/// `"Source"` for level 0, `"LOD 1"` and up for the rest — matching the names the
/// processed meshes and the export carry.
fn lod_label(level: usize) -> String {
    if level == 0 {
        review_localization::tr(keys::ui_status_bar::LOD_FULL).into_owned()
    } else {
        keys::ui_status_bar::lod_level(level as f64)
    }
}

/// The Texture-workspace status bar: the texture-stats toggle inset on the left
/// (the Tex viewport's analogue of the model-stats info button) and the
/// background-fill radio group (Black / White / Grey / Checker) mirrored to the
/// right. Laid out with the same edge inset as the 3D bar.
fn draw_texture_status_bar(ui: &mut egui::Ui, state: &mut UiState, group_height: f32) {
    let single_icon_group_width = size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH;
    let bg_group_width = size::TEXTURE_BG_GROUP_WIDTH;
    let segment_w = size::TEXTURE_BG_SEGMENT_WIDTH;

    let bar_rect = ui.max_rect();
    let edge_inset = (bar_rect.height() - group_height) * 0.5;

    // Texture-stats toggle — single-icon group inset equally on the left.
    let left_rect = bar_group_rect(
        bar_rect,
        false,
        edge_inset,
        single_icon_group_width,
        group_height,
    );
    bar_group_scope(ui, left_rect, group_height, |ui| {
        toolbar_group_shell(ui, single_icon_group_width, |ui| {
            if icon_toggle_button(
                ui,
                &ICON_INFO,
                state.texture_view.show_stats,
                Tip::new(keys::ui_status_bar::TEXTURE_INFO)
                    .describe(keys::ui_status_bar::TEXTURE_INFO_DESCRIPTION)
                    .page(Page::Tex),
            )
            .clicked()
            {
                state.texture_view.show_stats = !state.texture_view.show_stats;
            }
        });
    });

    // Zoom-level readout next to the texture-info button: the current zoom as an
    // integer percentage, clickable to reset to 100%.
    let zoom_label_width = size::TEXTURE_ZOOM_LABEL_WIDTH;
    let zoom_rect = egui::Rect::from_min_size(
        egui::pos2(
            left_rect.right() + edge_inset,
            bar_rect.center().y - group_height * 0.5,
        ),
        egui::vec2(zoom_label_width, group_height),
    );
    bar_group_scope(ui, zoom_rect, group_height, |ui| {
        if zoom_reset_label(ui, state.texture_view.zoom).clicked() {
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
    });

    // Background-fill radio group — mirrored to the right edge.
    let right_rect = bar_group_rect(bar_rect, true, edge_inset, bg_group_width, group_height);
    bar_group_scope(ui, right_rect, group_height, |ui| {
        toolbar_group_shell(ui, bg_group_width, |ui| {
            for background in TextureBackground::ALL {
                let selected = state.texture_view.background == background;
                let response = tip(
                    segment_button(
                        ui,
                        review_localization::tr(labels::texture_background(background)).as_ref(),
                        selected,
                        segment_w,
                    ),
                    background_tooltip(background),
                );
                if response.clicked() {
                    state.texture_view.background = background;
                }
            }
        });
    });
}

/// A clickable zoom-percentage readout for the Tex status bar: shows the current
/// zoom rounded to the nearest integer percent (e.g. `120%`), brightening on
/// hover; clicking it toggles between 100% and fit-to-view.
fn zoom_reset_label(ui: &mut egui::Ui, zoom: f32) -> egui::Response {
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
        keys::ui_status_bar::zoom_percent(f64::from(percent)),
        egui::FontId::monospace(font::STATUS_ZOOM),
        text_color,
    );
    tip(
        response,
        Tip::new(keys::ui_status_bar::ZOOM)
            .describe(keys::ui_status_bar::ZOOM_DESCRIPTION)
            .page(Page::Tex),
    )
}

/// Hover tooltip spelling out a background-fill segment's single-letter label.
fn background_tooltip(background: TextureBackground) -> Tip {
    let title = match background {
        TextureBackground::Black => keys::ui_status_bar::BACKGROUND_BLACK,
        TextureBackground::White => keys::ui_status_bar::BACKGROUND_WHITE,
        TextureBackground::Grey => keys::ui_status_bar::BACKGROUND_GREY,
        TextureBackground::Checker => keys::ui_status_bar::BACKGROUND_CHECKER,
    };
    let tip = Tip::new(title).page(Page::Tex);
    // Only the checker has anything to add: the other three are what their names
    // say, and a paragraph restating "this makes the background black" is noise.
    match background {
        TextureBackground::Checker => {
            tip.describe(keys::ui_status_bar::BACKGROUND_CHECKER_DESCRIPTION)
        }
        _ => tip,
    }
}
