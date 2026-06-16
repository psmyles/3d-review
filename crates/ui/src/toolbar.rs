//! The top toolbar: shading-mode group, debug-view group, the 3D/UV/Tex mode
//! segments, and the gizmo/grid/projection group. Emits panel-open intents by
//! mutating [`UiState`] in place.

use review_render::{ActiveMaterial, ShadingMode, UvShadingMode};

use crate::assets::{
    ICON_AXIS_GIZMO, ICON_BBOX, ICON_GRID, ICON_NORMALS_FACE, ICON_NORMALS_VERTEX,
    ICON_SHADING_SHADED, ICON_SHADING_TEXTURE, ICON_SHADING_UNLIT, ICON_SHADING_WIRE,
    ICON_SHADING_WIRE_ONLY, ICON_UV, ICON_UV_ISLANDS, ICON_UV_SHADED, ICON_UV_WIRE,
    ICON_VERTEX_COLORS, ICON_VIEW_ORTHO, ICON_VIEW_PERSPECTIVE,
};
use crate::state::{OptionPanel, UiState, ViewProjectionMode, WorkspaceMode};
use crate::theme::{self, color, font, size};
use crate::widgets::{compact_combo, icon_toggle_button, toolbar_group_shell};

/// Background frame shared by the toolbar (and matched by the status bar). Zero
/// inner margin: content is placed by px-converted rect math below, so no raw
/// pixel literals leak into the frame.
pub(crate) fn toolbar_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(color::CHROME_BG)
        .stroke(egui::Stroke::NONE)
        .inner_margin(egui::Margin::same(0))
}

pub(crate) fn draw(ctx: &egui::Context, state: &mut UiState) {
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
    let mode_group_width = theme::px(ctx, size::TOOLBAR_MODE_GROUP_WIDTH);

    egui::TopBottomPanel::top("app_toolbar")
        .exact_height(toolbar_height)
        .frame(toolbar_frame())
        .show(ctx, |ui| {
            let bar_rect = ui.max_rect();
            ui.painter().line_segment(
                [
                    egui::pos2(bar_rect.left(), bar_rect.bottom() - 0.5),
                    egui::pos2(bar_rect.right(), bar_rect.bottom() - 0.5),
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
            // in the 3D workspace. UV mode replaces the right cluster with the
            // UV-set picker; Texture mode (placeholder) shows neither.
            if state.mode == WorkspaceMode::ThreeD {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(left_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.set_height(group_height);
                        ui.spacing_mut().item_spacing.x = group_spacing;
                        draw_shading_group(ui, ctx, state, shading_group_width);
                        draw_material_group(ui, ctx, state, material_group_width);
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
                        WorkspaceMode::ThreeD => {
                            draw_view_group(ui, ctx, state, triple_icon_group_width);
                            draw_projection_group(ui, ctx, state, single_icon_group_width);
                        }
                        WorkspaceMode::Uv => draw_uv_set_picker(ui, ctx, state),
                        WorkspaceMode::Texture => {}
                    }
                },
            );
        });
}

/// Shading group: the independent "Show Wireframe" overlay toggle followed by
/// the mutually-exclusive shading modes (wireframe-only / unlit / shaded). The
/// overlay can be on regardless of which shading mode is selected.
fn draw_shading_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        // 1. Show Wireframe — independent overlay toggle; retains its options panel.
        let wire_overlay = icon_toggle_button(
            ui,
            ctx,
            &ICON_SHADING_WIRE,
            state.debug.wireframe_overlay,
            "Show Wireframe (right-click for options)",
        );
        if wire_overlay.clicked() {
            state.debug.wireframe_overlay = !state.debug.wireframe_overlay;
        }
        if wire_overlay.secondary_clicked() {
            state.open_panel(OptionPanel::Wireframe);
        }

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
        if icon_toggle_button(ui, ctx, &ICON_SHADING_SHADED, shaded, "Shaded").clicked() {
            state.shading_mode = ShadingMode::Shaded;
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
/// show — source material, UV checker, or vertex colors. The UV-checker and
/// vertex-color buttons each retain their right-click options panel.
fn draw_material_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        let source = state.debug.active_material == ActiveMaterial::Source;
        let uv_active = state.debug.active_material == ActiveMaterial::UvChecker;
        let vertex_colors_active = state.debug.active_material == ActiveMaterial::VertexColors;

        if icon_toggle_button(ui, ctx, &ICON_SHADING_TEXTURE, source, "Source Material").clicked() {
            state.debug.active_material = ActiveMaterial::Source;
        }

        let uv = icon_toggle_button(
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
            state.open_panel(OptionPanel::UvChecker);
        }

        let vertex_colors = icon_toggle_button(
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
            state.open_panel(OptionPanel::VertexColors);
        }
    });
}

/// Normal-debug group: the face- and vertex-normal line overlays. Independent
/// toggles — any combination can be active. Each retains its options panel.
fn draw_normals_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        let face = icon_toggle_button(
            ui,
            ctx,
            &ICON_NORMALS_FACE,
            state.debug.face_normals,
            "Face Normal (right-click for options)",
        );
        if face.clicked() {
            state.debug.face_normals = !state.debug.face_normals;
        }
        if face.secondary_clicked() {
            state.open_panel(OptionPanel::FaceNormals);
        }

        let vertex = icon_toggle_button(
            ui,
            ctx,
            &ICON_NORMALS_VERTEX,
            state.debug.vertex_normals,
            "Vertex Normal (right-click for options)",
        );
        if vertex.clicked() {
            state.debug.vertex_normals = !state.debug.vertex_normals;
        }
        if vertex.secondary_clicked() {
            state.open_panel(OptionPanel::VertexNormals);
        }
    });
}

fn draw_view_group(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, ctx, width, |ui| {
        let bbox = icon_toggle_button(
            ui,
            ctx,
            &ICON_BBOX,
            state.debug.show_bounding_box,
            "Bounding Box (right-click for options)",
        );
        if bbox.clicked() {
            state.debug.show_bounding_box = !state.debug.show_bounding_box;
        }
        if bbox.secondary_clicked() {
            state.open_panel(OptionPanel::BoundingBox);
        }

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
}

fn mode_segment(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    mode: &mut WorkspaceMode,
    value: WorkspaceMode,
    label: &str,
) {
    let selected = *mode == value;
    let desired = egui::vec2(
        theme::px(ctx, size::MODE_SEGMENT_WIDTH),
        theme::px(ctx, size::MODE_SEGMENT_HEIGHT),
    );
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    let fill = if selected {
        color::ACCENT
    } else if response.hovered() {
        color::HOVER_BG
    } else {
        egui::Color32::TRANSPARENT
    };
    let text_color = if selected {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_SEGMENT_IDLE
    };

    ui.painter().rect(
        rect,
        theme::px(ctx, size::TILE_CORNER_RADIUS),
        fill,
        egui::Stroke::NONE,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(theme::px(ctx, font::MODE_SEGMENT)),
        text_color,
    );

    if response.clicked() {
        *mode = value;
    }
}
