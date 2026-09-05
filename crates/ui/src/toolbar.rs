//! The top toolbar: shading-mode group, debug-view group, the 3D/UV/Tex mode
//! segments, and the gizmo/grid/projection group. Emits panel-open intents by
//! mutating [`UiState`] in place.

use review_render::{ActiveMaterial, ShadingMode, UvShadingMode};

use crate::assets::{
    ICON_AXIS_GIZMO, ICON_BACKFACE, ICON_BBOX, ICON_BUFFERS, ICON_GRID, ICON_NODE_BONE,
    ICON_NORMALS_FACE, ICON_NORMALS_VERTEX, ICON_OUTLINER, ICON_PIVOT, ICON_SHADING_SHADED,
    ICON_SHADING_TEXTURE, ICON_SHADING_UNLIT, ICON_SHADING_WIRE, ICON_SHADING_WIRE_ONLY,
    ICON_SKIN_WEIGHTS, ICON_UV, ICON_UV_ISLANDS, ICON_UV_SHADED, ICON_UV_WIRE, ICON_VERTEX_COLORS,
    ICON_VIEW_ORTHO, ICON_VIEW_PERSPECTIVE,
};
use crate::state::{
    OptionPanel, TextureChannelView, TexturePoolEntry, UiState, ViewProjectionMode, WorkspaceMode,
};
use crate::theme::{self, color, size};
use crate::widgets::{
    compact_combo, icon_toggle_button, icon_toggle_button_with_options, option_toggle,
    segment_button, toolbar_group_shell,
};

/// Background frame shared by the toolbar (and matched by the status bar). Zero
/// inner margin: content is placed by px-converted rect math below, so no raw
/// pixel literals leak into the frame.
pub(crate) fn toolbar_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(color::CHROME_BG)
        .stroke(egui::Stroke::NONE)
        .inner_margin(egui::Margin::same(0))
}

pub(crate) fn draw(root: &mut egui::Ui, state: &mut UiState) {
    let ctx = &root.ctx().clone();
    let toolbar_height = theme::px(ctx, size::TOOLBAR_HEIGHT);
    let overlay_margin = theme::px(ctx, size::OVERLAY_MARGIN);
    let group_spacing = theme::px(ctx, size::TOOLBAR_GROUP_SPACING);
    let group_height = theme::px(ctx, size::TOOLBAR_GROUP_HEIGHT);
    let left_width = theme::px(ctx, size::TOOLBAR_LEFT_WIDTH);
    let center_width = theme::px(ctx, size::TOOLBAR_CENTER_WIDTH);
    let right_width = theme::px(ctx, size::TOOLBAR_RIGHT_WIDTH);
    let shading_group_width = theme::px(ctx, size::TOOLBAR_SHADING_GROUP_WIDTH);
    let material_group_width = theme::px(ctx, size::TOOLBAR_MATERIAL_GROUP_WIDTH);
    let normals_group_width = theme::px(ctx, size::TOOLBAR_NORMALS_GROUP_WIDTH);
    let single_icon_group_width = theme::px(ctx, size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH);
    let triple_icon_group_width = theme::px(ctx, size::TOOLBAR_TRIPLE_ICON_GROUP_WIDTH);
    let quad_icon_group_width = theme::px(ctx, size::TOOLBAR_QUAD_ICON_GROUP_WIDTH);
    let quint_icon_group_width = theme::px(ctx, size::TOOLBAR_QUINT_ICON_GROUP_WIDTH);
    let mode_group_width = theme::px(ctx, size::TOOLBAR_MODE_GROUP_WIDTH);

    egui::Panel::top("app_toolbar")
        .exact_size(toolbar_height)
        .frame(toolbar_frame())
        .show(root, |ui| {
            let bar_rect = ui.max_rect();
            ui.painter().line_segment(
                [
                    egui::pos2(bar_rect.left(), bar_rect.bottom() - size::HAIRLINE_NUDGE),
                    egui::pos2(bar_rect.right(), bar_rect.bottom() - size::HAIRLINE_NUDGE),
                ],
                egui::Stroke::new(size::HAIRLINE, color::DIVIDER),
            );
            let row_rect = egui::Rect::from_min_size(
                egui::pos2(
                    bar_rect.left() + overlay_margin,
                    bar_rect.top() + overlay_margin,
                ),
                egui::vec2(
                    (bar_rect.width() - overlay_margin * 2.0).max(0.0),
                    group_height,
                ),
            );
            let left_rect = egui::Rect::from_min_size(
                row_rect.left_top(),
                egui::vec2(left_width.min(row_rect.width()), group_height),
            );
            let center_rect = egui::Rect::from_center_size(
                egui::pos2(row_rect.center().x, row_rect.top() + group_height * 0.5),
                egui::vec2(center_width.min(row_rect.width()), group_height),
            );
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(
                    row_rect.right() - right_width.min(row_rect.width()),
                    row_rect.top(),
                ),
                egui::vec2(right_width.min(row_rect.width()), group_height),
            );

            // The shading / material / normal tool groups and the view /
            // projection groups operate on the 3D scene, so they are shown only
            // in the 3D workspace. UV mode swaps in the UV-shading group + UV-set
            // picker; Texture mode swaps in the channel group + texture picker.
            if state.mode.is_scene() {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(left_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.set_height(group_height);
                        ui.spacing_mut().item_spacing.x = group_spacing;
                        draw_shading_group(ui, ctx, state, shading_group_width);
                        // One tile wider when the model carries skin weights, so
                        // the extra radio has room.
                        let material_width = if state.has_skin {
                            quint_icon_group_width
                        } else {
                            material_group_width
                        };
                        draw_material_group(ui, ctx, state, material_width);
                        draw_normals_group(ui, ctx, state, normals_group_width);
                    },
                );
            } else if state.mode == WorkspaceMode::Uv {
                // UV mode swaps the 3D tool groups for the UV-shading group.
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(left_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.set_height(group_height);
                        ui.spacing_mut().item_spacing.x = group_spacing;
                        draw_uv_shading_group(ui, ctx, state, triple_icon_group_width);
                    },
                );
            } else if state.mode == WorkspaceMode::Texture {
                // Texture mode shows the channel radio group (RGB/R/G/B/A).
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(left_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.set_height(group_height);
                        ui.spacing_mut().item_spacing.x = group_spacing;
                        draw_texture_channel_group(ui, ctx, state);
                    },
                );
            }

            ui.scope_builder(
                egui::UiBuilder::new().max_rect(center_rect).layout(
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                ),
                |ui| {
                    ui.set_height(group_height);
                    toolbar_group_shell(ui, ctx, mode_group_width, |ui| {
                        segmented_mode_control(ui, ctx, &mut state.mode);
                    });
                },
            );

            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.set_height(group_height);
                    ui.spacing_mut().item_spacing.x = group_spacing;
                    match state.mode {
                        WorkspaceMode::ThreeD | WorkspaceMode::Opt => {
                            // The skeleton toggle only exists for a rigged model, so
                            // the group is one tile narrower without it.
                            let view_group_width = if state.has_bones {
                                quint_icon_group_width
                            } else {
                                quad_icon_group_width
                            };
                            draw_view_group(ui, ctx, state, view_group_width);
                            draw_projection_group(ui, ctx, state, single_icon_group_width);
                            draw_windows_group(ui, ctx, state, single_icon_group_width);
                        }
                        WorkspaceMode::Uv => draw_uv_set_picker(ui, ctx, state),
                        WorkspaceMode::Texture => draw_texture_picker(ui, ctx, state),
                    }
                },
            );
        });
}

/// Shading group: the independent "Show Wireframe" overlay toggle, the
/// mutually-exclusive shading modes (wireframe-only / unlit / shaded), and the
/// independent "Backface Rendering" toggle. The two toggles bookend the radio:
/// either can be on regardless of which shading mode is selected.
fn draw_shading_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        // 1. Show Wireframe — independent overlay toggle; retains its options panel.
        option_toggle(
            ui,
            ctx,
            &ICON_SHADING_WIRE,
            &mut state.debug.wireframe_overlay,
            &mut state.panels_open,
            OptionPanel::Wireframe,
            "Show Wireframe (right-click for options)",
        );

        // 2-4. Shading mode — radio selection; exactly one is active.
        let wire_only = matches!(state.shading_mode, ShadingMode::Wireframe);
        let unlit = matches!(state.shading_mode, ShadingMode::Unlit);
        let shaded = matches!(state.shading_mode, ShadingMode::Shaded);

        if icon_toggle_button(
            ui,
            ctx,
            &ICON_SHADING_WIRE_ONLY,
            wire_only,
            "Wireframe Only",
        )
        .clicked()
        {
            state.shading_mode = ShadingMode::Wireframe;
        }
        if icon_toggle_button(ui, ctx, &ICON_SHADING_UNLIT, unlit, "Unlit").clicked() {
            state.shading_mode = ShadingMode::Unlit;
        }
        // Shaded mode is environment-lit PBR; its IBL on/off + environment
        // options live on the dedicated IBL button in the status bar.
        if icon_toggle_button(ui, ctx, &ICON_SHADING_SHADED, shaded, "Shaded").clicked() {
            state.shading_mode = ShadingMode::Shaded;
        }

        // 5. Backface Rendering — independent toggle; off (default) culls back
        // faces, on draws the mesh double-sided.
        if icon_toggle_button(
            ui,
            ctx,
            &ICON_BACKFACE,
            state.debug.render_backfaces,
            "Backface Rendering",
        )
        .clicked()
        {
            state.debug.render_backfaces = !state.debug.render_backfaces;
        }
    });
}

/// UV-shading group (UV mode only): a radio selection of how the 2D UV view
/// shades the layout — wire-only, solid-shaded islands, or a unique color per
/// island. Exactly one is active; the UV edges are drawn in every mode.
fn draw_uv_shading_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        let wire = state.uv_shading_mode == UvShadingMode::Wire;
        let shaded = state.uv_shading_mode == UvShadingMode::Shaded;
        let islands = state.uv_shading_mode == UvShadingMode::Islands;

        if icon_toggle_button(ui, ctx, &ICON_UV_WIRE, wire, "UV Wire").clicked() {
            state.uv_shading_mode = UvShadingMode::Wire;
        }
        if icon_toggle_button(ui, ctx, &ICON_UV_SHADED, shaded, "UV Shaded").clicked() {
            state.uv_shading_mode = UvShadingMode::Shaded;
        }
        if icon_toggle_button(ui, ctx, &ICON_UV_ISLANDS, islands, "UV Islands").clicked() {
            state.uv_shading_mode = UvShadingMode::Islands;
        }
    });
}

/// Active-material group: a radio selection of which material the filled faces
/// show — source material, UV checker, vertex colors, a buffer-inspection view,
/// or (for a skinned model only) the skin-weight heat map. The UV-checker,
/// vertex-color and buffers buttons each retain their right-click options panel.
fn draw_material_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        let source = state.debug.active_material == ActiveMaterial::Source;
        let uv_active = state.debug.active_material == ActiveMaterial::UvChecker;
        let vertex_colors_active = state.debug.active_material == ActiveMaterial::VertexColors;
        let buffers_active = state.debug.active_material == ActiveMaterial::Buffers;

        let source_material = icon_toggle_button_with_options(
            ui,
            ctx,
            &ICON_SHADING_TEXTURE,
            source,
            "Source Material (click to cycle modes, right-click for options)",
        );
        if source_material.clicked() {
            // First click activates the source-material shading; clicking again
            // while already active cycles Source -> Standard -> Unique -> Source.
            if source {
                state.debug.material_mode = state.debug.material_mode.next();
            } else {
                state.debug.active_material = ActiveMaterial::Source;
            }
        }
        if source_material.secondary_clicked() {
            state.panels_open.toggle(OptionPanel::MaterialMode);
        }

        let uv = icon_toggle_button_with_options(
            ui,
            ctx,
            &ICON_UV,
            uv_active,
            "UV Checker (right-click for options)",
        );
        if uv.clicked() {
            state.debug.active_material = ActiveMaterial::UvChecker;
        }
        if uv.secondary_clicked() {
            state.panels_open.toggle(OptionPanel::UvChecker);
        }

        let vertex_colors = icon_toggle_button_with_options(
            ui,
            ctx,
            &ICON_VERTEX_COLORS,
            vertex_colors_active,
            "Vertex Colors (right-click for options)",
        );
        if vertex_colors.clicked() {
            state.debug.active_material = ActiveMaterial::VertexColors;
        }
        if vertex_colors.secondary_clicked() {
            state.panels_open.toggle(OptionPanel::VertexColors);
        }

        let buffers = icon_toggle_button_with_options(
            ui,
            ctx,
            &ICON_BUFFERS,
            buffers_active,
            "Buffers (click to cycle buffers, right-click for options)",
        );
        if buffers.clicked() {
            // First click activates the buffer-inspection view; clicking again
            // while already active cycles through the individual buffers.
            if buffers_active {
                state.debug.buffer_view = state.debug.buffer_view.next();
            } else {
                state.debug.active_material = ActiveMaterial::Buffers;
            }
        }
        if buffers.secondary_clicked() {
            state.panels_open.toggle(OptionPanel::BufferView);
        }

        // Skin weights — offered only for a model that actually carries them, the
        // same rule as the skeleton toggle. No options panel: the ramp is fixed
        // and what it paints is chosen in the Outliner.
        if state.has_skin {
            let weights_active = state.debug.active_material == ActiveMaterial::SkinWeights;
            if icon_toggle_button(
                ui,
                ctx,
                &ICON_SKIN_WEIGHTS,
                weights_active,
                "Skin Weights (select bones in the Outliner)",
            )
            .clicked()
            {
                state.debug.active_material = ActiveMaterial::SkinWeights;
            }
        }
    });
}

/// Normal-debug group: the face- and vertex-normal line overlays. Independent
/// toggles — any combination can be active. Each retains its options panel.
fn draw_normals_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        option_toggle(
            ui,
            ctx,
            &ICON_NORMALS_FACE,
            &mut state.debug.face_normals,
            &mut state.panels_open,
            OptionPanel::FaceNormals,
            "Face Normal (right-click for options)",
        );
        option_toggle(
            ui,
            ctx,
            &ICON_NORMALS_VERTEX,
            &mut state.debug.vertex_normals,
            &mut state.panels_open,
            OptionPanel::VertexNormals,
            "Vertex Normal (right-click for options)",
        );
    });
}

/// View group: the bounding box (with its options panel), the object pivot marker,
/// the axis gizmo, and the floor grid. The bounding box and pivot draw in the 3D
/// scene; the gizmo and grid are independent display toggles.
fn draw_view_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        option_toggle(
            ui,
            ctx,
            &ICON_BBOX,
            &mut state.debug.show_bounding_box,
            &mut state.panels_open,
            OptionPanel::BoundingBox,
            "Bounding Box (right-click for options)",
        );

        // Skeleton overlay — hidden entirely for a model with no bones, rather
        // than shown disabled: an unrigged mesh has nothing to say about it.
        if state.has_bones {
            option_toggle(
                ui,
                ctx,
                &ICON_NODE_BONE,
                &mut state.debug.show_skeleton,
                &mut state.panels_open,
                OptionPanel::Skeleton,
                "Skeleton (right-click for options)",
            );
        }

        // Pivot marker — a plain on/off toggle (no options), sitting between the
        // bounding box and the axis gizmo.
        icon_toggle_button(ui, ctx, &ICON_PIVOT, state.debug.show_pivot, "Pivot")
            .clicked()
            .then(|| state.debug.show_pivot = !state.debug.show_pivot);

        icon_toggle_button(
            ui,
            ctx,
            &ICON_AXIS_GIZMO,
            state.show_axis_gizmo,
            "Axis Gizmo",
        )
        .clicked()
        .then(|| state.show_axis_gizmo = !state.show_axis_gizmo);

        icon_toggle_button(ui, ctx, &ICON_GRID, state.show_grid, "Grid")
            .clicked()
            .then(|| state.show_grid = !state.show_grid);
    });
}

/// Side-panel toggle group (3D mode): one button opening the Outliner (left) and
/// the Inspector (right) together. They are two halves of one workflow — the
/// Outliner picks a row, the Inspector describes it — so they share a toggle, and
/// it shows highlighted while they're open.
fn draw_windows_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        if icon_toggle_button(
            ui,
            ctx,
            &ICON_OUTLINER,
            state.side_panels_open,
            "Outliner & Inspector",
        )
        .clicked()
        {
            state.side_panels_open = !state.side_panels_open;
        }
    });
}

fn draw_projection_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        let (icon, tooltip) = match state.projection_mode {
            ViewProjectionMode::Perspective => (
                &ICON_VIEW_PERSPECTIVE,
                "Perspective camera (click for orthographic)",
            ),
            ViewProjectionMode::Orthographic => (
                &ICON_VIEW_ORTHO,
                "Orthographic camera (click for perspective)",
            ),
        };

        if icon_toggle_button(ui, ctx, icon, false, tooltip).clicked() {
            state.projection_mode = match state.projection_mode {
                ViewProjectionMode::Perspective => ViewProjectionMode::Orthographic,
                ViewProjectionMode::Orthographic => ViewProjectionMode::Perspective,
            };
        }
    });
}

/// The UV-set dropdown shown on the right of the toolbar in UV mode: lists the
/// model's UV sets in source-file order and selects which one the UV view draws.
/// Hidden when the model carries no UV sets.
fn draw_uv_set_picker(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState) {
    if state.uv_sets.is_empty() {
        return;
    }
    // Keep the selected channel in range (a reload may have shrunk the set list).
    let count = state.uv_sets.len() as u32;
    if state.uv_view_channel >= count {
        state.uv_view_channel = 0;
    }

    // Route through the shared combo helper so the closed button and its popup
    // read identically to the option-panel dropdowns (fill, rounding, padding,
    // font, and the no-blue-fill selection treatment).
    let width = theme::px(ctx, size::TOOLBAR_UV_DROPDOWN_WIDTH);
    let labels = state.uv_sets.clone();
    let selected = labels[state.uv_view_channel as usize].clone();
    compact_combo(ui, "uv_set_picker", width, selected, |ui| {
        for (channel, label) in labels.iter().enumerate() {
            ui.selectable_value(&mut state.uv_view_channel, channel as u32, label);
        }
    });
}

fn segmented_mode_control(ui: &mut egui::Ui, ctx: &egui::Context, mode: &mut WorkspaceMode) {
    mode_segment(ui, ctx, mode, WorkspaceMode::ThreeD, "3D");
    mode_segment(ui, ctx, mode, WorkspaceMode::Uv, "UV");
    mode_segment(ui, ctx, mode, WorkspaceMode::Texture, "Tex");
    mode_segment(ui, ctx, mode, WorkspaceMode::Opt, "Opt");
}

fn mode_segment(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    mode: &mut WorkspaceMode,
    value: WorkspaceMode,
    label: &str,
) {
    let width = theme::px(ctx, size::MODE_SEGMENT_WIDTH);
    if segment_button(ui, ctx, label, *mode == value, width).clicked() {
        *mode = value;
    }
}

/// Channel radio group (RGB / R / G / B / A) shown on the left of the toolbar in
/// Texture mode: selects which channel of the viewed texture the Tex viewport
/// displays. Exactly one is active. The `A` segment is shown only when the viewed
/// image actually carries an alpha channel; an opaque (RGB / greyscale) source
/// hides it, and the group shrinks to the remaining segments.
fn draw_texture_channel_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState) {
    // Alpha exists when the decoded source had 2 (grey+a) or 4 (RGBA) channels.
    let has_alpha = state
        .texture_pool
        .get(state.texture_view.selected)
        .map(|entry| matches!(entry.image.source_channels, 2 | 4))
        .unwrap_or(false);
    // If alpha was the active channel but the new image has none, fall back to
    // the full-RGB view so the now-hidden A segment isn't left selected.
    if !has_alpha && state.texture_view.channel == TextureChannelView::A {
        state.texture_view.channel = TextureChannelView::Rgb;
    }

    let segment_w = theme::px(ctx, size::TEXTURE_CHANNEL_SEGMENT_WIDTH);
    let padding = theme::px(ctx, size::TOOLBAR_GROUP_PADDING);
    let gap = theme::px(ctx, size::TOOLBAR_ICON_GAP);
    let count = if has_alpha { 5.0 } else { 4.0 };
    // Size the shell to exactly the visible segments (matching its own
    // padding/gap), so dropping A tightens the group instead of leaving a gap.
    let width = padding * 2.0 + count * segment_w + (count - 1.0) * gap;
    toolbar_group_shell(ui, ctx, width, |ui| {
        for channel in TextureChannelView::ALL {
            if channel == TextureChannelView::A && !has_alpha {
                continue;
            }
            let selected = state.texture_view.channel == channel;
            if segment_button(ui, ctx, channel.label(), selected, segment_w).clicked() {
                state.texture_view.channel = channel;
            }
        }
    });
}

/// The texture-picker dropdown shown on the right of the toolbar in Texture mode:
/// lists the scene texture pool by file name and selects which one the Tex
/// viewport shows. Hidden when the pool is empty.
fn draw_texture_picker(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState) {
    if state.texture_pool.is_empty() {
        return;
    }
    // Keep the selection in range (a removed texture may have shrunk the pool).
    if state.texture_view.selected >= state.texture_pool.len() {
        state.texture_view.selected = 0;
    }

    let width = theme::px(ctx, size::TOOLBAR_TEXTURE_DROPDOWN_WIDTH);
    let names: Vec<String> = state
        .texture_pool
        .iter()
        .map(TexturePoolEntry::name)
        .collect();
    let selected = names[state.texture_view.selected].clone();
    compact_combo(ui, "texture_picker", width, selected, |ui| {
        for (index, name) in names.iter().enumerate() {
            ui.selectable_value(&mut state.texture_view.selected, index, name);
        }
    });
}
