//! The top toolbar: the application menu, the wireframe overlay, the shading-mode
//! group, debug-view group, the 3D/UV/Tex mode segments, and the
//! gizmo/grid/projection group, with the side-panels toggle and then Help closing
//! out the right-hand end. Emits panel-open intents by mutating [`UiState`] in
//! place, and the menu's commands as a [`MenuIntent`].

use review_render::{ActiveMaterial, ShadingMode, UvShadingMode};

use crate::assets::{
    ICON_AXIS_GIZMO, ICON_BACKFACE, ICON_BBOX, ICON_BUFFERS, ICON_COMMENT, ICON_GRID, ICON_HELP,
    ICON_MENU, ICON_NODE_BONE, ICON_NORMALS_FACE, ICON_NORMALS_VERTEX, ICON_OUTLINER, ICON_PIVOT,
    ICON_SELECT, ICON_SHADING_SHADED, ICON_SHADING_TEXTURE, ICON_SHADING_UNLIT, ICON_SHADING_WIRE,
    ICON_SHADING_WIRE_ONLY, ICON_SKIN_WEIGHTS, ICON_UV, ICON_UV_ISLANDS, ICON_UV_SEAM,
    ICON_UV_SHADED, ICON_UV_WIRE, ICON_VERTEX_COLORS, ICON_VIEW_ORTHO, ICON_VIEW_PERSPECTIVE,
};
use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::{
    MenuIntent, OptionPanel, TextureChannelView, UiOutput, UiState, ViewProjectionMode,
    ViewportTool, WorkspaceMode,
};
use crate::theme::{color, size};
use crate::widgets::{
    Tip, compact_combo, icon_toggle_button, icon_toggle_button_with_options, option_toggle,
    segment_button, tip, toolbar_group_shell,
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

pub(crate) fn draw(root: &mut egui::Ui, state: &mut UiState, output: &mut UiOutput) {
    let toolbar_height = size::TOOLBAR_HEIGHT;
    let overlay_margin = size::OVERLAY_MARGIN;
    let group_spacing = size::TOOLBAR_GROUP_SPACING;
    let group_height = size::TOOLBAR_GROUP_HEIGHT;
    let left_width = size::TOOLBAR_LEFT_WIDTH;
    let center_width = size::TOOLBAR_CENTER_WIDTH;
    let right_width = size::TOOLBAR_RIGHT_WIDTH;
    let shading_group_width = size::TOOLBAR_SHADING_GROUP_WIDTH;
    let material_group_width = size::TOOLBAR_MATERIAL_GROUP_WIDTH;
    let single_icon_group_width = size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH;
    let triple_icon_group_width = size::TOOLBAR_TRIPLE_ICON_GROUP_WIDTH;
    let quad_icon_group_width = size::TOOLBAR_QUAD_ICON_GROUP_WIDTH;
    let quint_icon_group_width = size::TOOLBAR_QUINT_ICON_GROUP_WIDTH;
    let mode_group_width = size::TOOLBAR_MODE_GROUP_WIDTH;

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

            // The menu anchors the left end of the bar in every workspace: its
            // commands are about the application, not the view. After it, the
            // shading / material / normal tool groups operate on the 3D scene, so
            // they are shown only in the 3D workspace. UV mode swaps in the
            // UV-shading group + UV-set picker; Texture mode swaps in the channel
            // group (its image is picked in the Outliner's Textures tab).
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_height(group_height);
                    ui.spacing_mut().item_spacing.x = group_spacing;
                    draw_menu_group(ui, state, output, single_icon_group_width);
                    match state.mode {
                        WorkspaceMode::ThreeD | WorkspaceMode::Opt => {
                            draw_wireframe_group(ui, state, single_icon_group_width);
                            draw_shading_group(ui, state, shading_group_width);
                            // One tile wider when the model carries skin weights,
                            // so the extra radio has room.
                            let material_width = if state.has_skin {
                                quint_icon_group_width
                            } else {
                                material_group_width
                            };
                            draw_material_group(ui, state, material_width);
                            draw_geometry_debug_group(ui, state, triple_icon_group_width);
                        }
                        WorkspaceMode::Uv => {
                            draw_uv_shading_group(ui, state, triple_icon_group_width);
                        }
                        WorkspaceMode::Texture => draw_texture_channel_group(ui, state),
                    }
                },
            );

            ui.scope_builder(
                egui::UiBuilder::new().max_rect(center_rect).layout(
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                ),
                |ui| {
                    ui.set_height(group_height);
                    toolbar_group_shell(ui, mode_group_width, |ui| {
                        segmented_mode_control(ui, &mut state.mode);
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
                            // Laid out right to left, so the first call is the
                            // rightmost tile: Help anchors the end of the bar.
                            draw_help_group(ui, state, single_icon_group_width);
                            draw_view_group(ui, state, view_group_width);
                            draw_projection_group(ui, state, single_icon_group_width);
                            draw_side_panels_group(ui, state, single_icon_group_width);
                            draw_tool_group(ui, state, size::TOOLBAR_DOUBLE_ICON_GROUP_WIDTH);
                        }
                        // The 2D workspaces keep Help and the side panels — the
                        // Outliner and Inspector are in every workspace — and UV
                        // adds the set it lays out. Tex picks its image from the
                        // Outliner's Textures tab.
                        WorkspaceMode::Uv => {
                            draw_help_group(ui, state, single_icon_group_width);
                            draw_side_panels_group(ui, state, single_icon_group_width);
                            draw_uv_set_picker(ui, state);
                        }
                        WorkspaceMode::Texture => {
                            draw_help_group(ui, state, single_icon_group_width);
                            draw_side_panels_group(ui, state, single_icon_group_width);
                        }
                    }
                },
            );
        });
}

/// The fixed id of the menu's popup, so the menu tile can ask whether it is open
/// before the tile itself — whose response the popup would otherwise be keyed
/// on — has been laid out.
const MENU_POPUP_ID: &str = "toolbar_menu";

/// The application menu, alone in a group at the far left of the bar.
///
/// Its own group, and first, for the same reason Help is alone and last: its
/// entries act on the application - the file, the settings, the process - rather
/// than on what is being looked at, so it reads as the thing before the tools
/// rather than one of them.
///
/// Four submenus, File / Preferences / Debug / Help, built from egui's own `menu_button`
/// (which becomes a submenu inside a menu). What reaches outside the chrome - a
/// dialog, the loaded model, the settings file, the network, the process -
/// travels to `app` as a [`MenuIntent`]; what the chrome can do itself (open the
/// About box, the manual or the log, hand a link to the browser) it does in place.
fn draw_menu_group(ui: &mut egui::Ui, state: &mut UiState, output: &mut UiOutput, width: f32) {
    let popup_id = egui::Id::new(MENU_POPUP_ID);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    toolbar_group_shell(ui, width, |ui| {
        let button = icon_toggle_button(
            ui,
            &ICON_MENU,
            open,
            Tip::new(keys::ui_toolbar::MENU)
                .describe(keys::ui_toolbar::MENU_DESCRIPTION)
                .page(Page::Menu),
        );
        egui::Popup::menu(&button).id(popup_id).show(|ui| {
            ui.menu_button(keys::ui_toolbar::MENU_FILE, |ui| {
                file_menu(ui, state, output);
            });
            ui.menu_button(keys::ui_toolbar::MENU_PREFERENCES, |ui| {
                preferences_menu(ui, state, output);
            });
            ui.menu_button(keys::ui_toolbar::MENU_DEBUG, |ui| {
                debug_menu(ui, state, output);
            });
            ui.menu_button(keys::ui_toolbar::MENU_HELP, |ui| {
                help_menu(ui, state, output);
            });
        });
    });
}

/// File: open, open recent, close, exit.
fn file_menu(ui: &mut egui::Ui, state: &UiState, output: &mut UiOutput) {
    let modifier = crate::primary_modifier().into_owned();
    let open_file = egui::Button::new(keys::ui_toolbar::MENU_OPEN_FILE)
        .shortcut_text(keys::ui_toolbar::menu_open_file_shortcut(modifier.clone()));
    if ui.add(open_file).clicked() {
        output.menu = Some(MenuIntent::OpenFile);
    }
    // Greyed out rather than hidden while there is no history, so the menu keeps
    // its shape.
    ui.add_enabled_ui(!state.recent_files.is_empty(), |ui| {
        ui.menu_button(keys::ui_toolbar::MENU_OPEN_RECENT, |ui| {
            recent_files_menu(ui, state, output);
        });
    });
    // Nothing to close until a model is loaded; `bounds` is `None` exactly then.
    let close_file = egui::Button::new(keys::ui_toolbar::MENU_CLOSE_FILE)
        .shortcut_text(keys::ui_toolbar::menu_close_file_shortcut(modifier.clone()));
    if ui.add_enabled(state.bounds.is_some(), close_file).clicked() {
        output.menu = Some(MenuIntent::CloseFile);
    }
    ui.separator();
    // The comments are the one part of the file the viewer changes, so they are
    // what Save writes — back into the FBX, or into a copy of it.
    let writable = state.comments.writable();
    let save = egui::Button::new(keys::ui_toolbar::MENU_SAVE)
        .shortcut_text(keys::ui_toolbar::menu_save_shortcut(modifier.clone()));
    if ui.add_enabled(writable, save).clicked() {
        output.menu = Some(MenuIntent::SaveComments);
    }
    let save_as = egui::Button::new(keys::ui_toolbar::MENU_SAVE_AS)
        .shortcut_text(keys::ui_toolbar::menu_save_as_shortcut(modifier.clone()));
    if ui.add_enabled(writable, save_as).clicked() {
        output.menu = Some(MenuIntent::SaveCommentsAs);
    }
    ui.separator();
    if ui.button(keys::ui_toolbar::MENU_EXIT).clicked() {
        output.menu = Some(MenuIntent::Exit);
    }
}

/// File > Open Recent: one entry per recent model, most recent first, named by
/// its file name with the full path on hover (two files of one name in different
/// folders are otherwise indistinguishable), then the command that empties it.
fn recent_files_menu(ui: &mut egui::Ui, state: &UiState, output: &mut UiOutput) {
    for (index, path) in state.recent_files.iter().enumerate() {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        if ui
            .button(name)
            .on_hover_text(path.display().to_string())
            .clicked()
        {
            output.menu = Some(MenuIntent::OpenRecent(index));
        }
    }
    ui.separator();
    if ui
        .button(keys::ui_toolbar::MENU_CLEAR_RECENT_FILES)
        .clicked()
    {
        output.menu = Some(MenuIntent::ClearRecentFiles);
    }
}

/// Preferences: the Remember Settings switch.
fn preferences_menu(ui: &mut egui::Ui, state: &UiState, output: &mut UiOutput) {
    // A copy, not the field: flipping it is `app`'s job, because flipping it is
    // also what writes the settings file.
    let mut remember = state.remember_settings;
    let remember_response = ui.checkbox(&mut remember, keys::ui_toolbar::MENU_REMEMBER_SETTINGS);
    if tip(
        remember_response,
        Tip::new(keys::ui_toolbar::MENU_REMEMBER_SETTINGS)
            .describe(keys::ui_toolbar::MENU_REMEMBER_SETTINGS_DESCRIPTION)
            .page(Page::Menu),
    )
    .clicked()
    {
        output.menu = Some(MenuIntent::ToggleRememberSettings);
    }
}

/// Debug: the log, and the Tracy Profiler switch. Things for finding out what
/// the viewer is doing, rather than for setting it up.
fn debug_menu(ui: &mut egui::Ui, state: &mut UiState, output: &mut UiOutput) {
    // Opens the window in place, like About: nothing outside the chrome changes.
    // `app` sees it open on the next frame and starts handing it lines.
    let view_log = ui.button(keys::ui_toolbar::MENU_VIEW_LOG);
    if tip(
        view_log,
        Tip::new(keys::ui_toolbar::MENU_VIEW_LOG)
            .describe(keys::ui_toolbar::MENU_VIEW_LOG_DESCRIPTION)
            .page(Page::Menu),
    )
    .clicked()
    {
        state.log.open = true;
    }

    // A copy, not the field, as with Remember Settings: the switch lives in the
    // settings file, and flipping it is `app`'s job.
    let mut tracy = state.tracy_profiler;
    let tracy_response = ui.checkbox(&mut tracy, keys::ui_toolbar::MENU_TRACY_PROFILER);
    if tip(
        tracy_response,
        Tip::new(keys::ui_toolbar::MENU_TRACY_PROFILER)
            .describe(keys::ui_toolbar::MENU_TRACY_PROFILER_DESCRIPTION)
            .page(Page::Menu),
    )
    .clicked()
    {
        output.menu = Some(MenuIntent::ToggleTracyProfiler);
    }
}

/// Help: about, the manual, updates, and the two project links.
fn help_menu(ui: &mut egui::Ui, state: &mut UiState, output: &mut UiOutput) {
    if ui.button(keys::ui_toolbar::MENU_ABOUT).clicked() {
        state.about.open = true;
    }
    if ui.button(keys::ui_toolbar::MENU_DOCUMENTATION).clicked() {
        state.help.open_contents();
    }
    ui.separator();
    if ui
        .button(keys::ui_toolbar::MENU_CHECK_FOR_UPDATES)
        .clicked()
    {
        output.menu = Some(MenuIntent::CheckForUpdates);
    }
    // Both links go to the browser through egui's own `open_url`, which
    // `egui-winit` hands to the OS.
    if ui.button(keys::ui_toolbar::MENU_REPORT_ISSUE).clicked() {
        ui.ctx()
            .open_url(egui::OpenUrl::new_tab(state.about.info.new_issue_url()));
    }
    if ui.button(keys::ui_toolbar::MENU_CREDITS).clicked() {
        ui.ctx()
            .open_url(egui::OpenUrl::new_tab(state.about.info.credits_url()));
    }
}

/// Show Wireframe, alone in a group: the independent wireframe-overlay toggle,
/// with its options panel. A group of its own rather than a tile in the shading
/// group, because it is not a shading mode — it draws over whichever one is
/// selected, and sitting beside the radio made it read as a fourth choice.
fn draw_wireframe_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        option_toggle(
            ui,
            &ICON_SHADING_WIRE,
            &mut state.debug.wireframe_overlay,
            &mut state.panels_open,
            OptionPanel::Wireframe,
            Tip::new(keys::ui_toolbar::SHOW_WIREFRAME)
                .describe(keys::ui_toolbar::SHOW_WIREFRAME_DESCRIPTION)
                .with_options()
                .page(Page::Shading),
        );
    });
}

/// Shading group: the mutually-exclusive shading modes (wireframe-only / unlit /
/// shaded), then the independent "Backface Rendering" toggle, which can be on
/// regardless of which shading mode is selected.
fn draw_shading_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        // 1-3. Shading mode — radio selection; exactly one is active.
        let wire_only = matches!(state.shading_mode, ShadingMode::Wireframe);
        let unlit = matches!(state.shading_mode, ShadingMode::Unlit);
        let shaded = matches!(state.shading_mode, ShadingMode::Shaded);

        if icon_toggle_button(
            ui,
            &ICON_SHADING_WIRE_ONLY,
            wire_only,
            Tip::new(keys::ui_toolbar::WIREFRAME_ONLY)
                .describe(keys::ui_toolbar::WIREFRAME_ONLY_DESCRIPTION)
                .page(Page::Shading),
        )
        .clicked()
        {
            state.shading_mode = ShadingMode::Wireframe;
        }
        if icon_toggle_button(
            ui,
            &ICON_SHADING_UNLIT,
            unlit,
            Tip::new(keys::ui_toolbar::UNLIT)
                .describe(keys::ui_toolbar::UNLIT_DESCRIPTION)
                .page(Page::Shading),
        )
        .clicked()
        {
            state.shading_mode = ShadingMode::Unlit;
        }
        // Shaded mode is environment-lit PBR; its IBL on/off + environment
        // options live on the dedicated IBL button in the status bar.
        if icon_toggle_button(
            ui,
            &ICON_SHADING_SHADED,
            shaded,
            Tip::new(keys::ui_toolbar::SHADED)
                .describe(keys::ui_toolbar::SHADED_DESCRIPTION)
                .page(Page::Shading),
        )
        .clicked()
        {
            state.shading_mode = ShadingMode::Shaded;
        }

        // 4. Backface Rendering — independent toggle; off (default) culls back
        // faces, on draws the mesh double-sided.
        if icon_toggle_button(
            ui,
            &ICON_BACKFACE,
            state.debug.render_backfaces,
            Tip::new(keys::ui_toolbar::BACKFACE_RENDERING)
                .describe(keys::ui_toolbar::BACKFACE_RENDERING_DESCRIPTION)
                .page(Page::Shading),
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
fn draw_uv_shading_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        let wire = state.uv_shading_mode == UvShadingMode::Wire;
        let shaded = state.uv_shading_mode == UvShadingMode::Shaded;
        let islands = state.uv_shading_mode == UvShadingMode::Islands;

        if icon_toggle_button(
            ui,
            &ICON_UV_WIRE,
            wire,
            Tip::new(keys::ui_toolbar::UV_WIRE)
                .describe(keys::ui_toolbar::UV_WIRE_DESCRIPTION)
                .page(Page::Uv),
        )
        .clicked()
        {
            state.uv_shading_mode = UvShadingMode::Wire;
        }
        if icon_toggle_button(
            ui,
            &ICON_UV_SHADED,
            shaded,
            Tip::new(keys::ui_toolbar::UV_SHADED)
                .describe(keys::ui_toolbar::UV_SHADED_DESCRIPTION)
                .page(Page::Uv),
        )
        .clicked()
        {
            state.uv_shading_mode = UvShadingMode::Shaded;
        }
        if icon_toggle_button(
            ui,
            &ICON_UV_ISLANDS,
            islands,
            Tip::new(keys::ui_toolbar::UV_ISLANDS)
                .describe(keys::ui_toolbar::UV_ISLANDS_DESCRIPTION)
                .page(Page::Uv),
        )
        .clicked()
        {
            state.uv_shading_mode = UvShadingMode::Islands;
        }
    });
}

/// Active-material group: a radio selection of which material the filled faces
/// show — source material, UV checker, vertex colors, a buffer-inspection view,
/// or (for a skinned model only) the skin-weight heat map. The UV-checker,
/// vertex-color and buffers buttons each retain their right-click options panel.
fn draw_material_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        let source = state.debug.active_material == ActiveMaterial::Source;
        let uv_active = state.debug.active_material == ActiveMaterial::UvChecker;
        let vertex_colors_active = state.debug.active_material == ActiveMaterial::VertexColors;
        let buffers_active = state.debug.active_material == ActiveMaterial::Buffers;

        let source_material = icon_toggle_button_with_options(
            ui,
            &ICON_SHADING_TEXTURE,
            source,
            Tip::new(keys::ui_toolbar::SOURCE_MATERIAL)
                .describe(keys::ui_toolbar::SOURCE_MATERIAL_DESCRIPTION)
                .cycles()
                .with_options()
                .page(Page::PanelsMaterialMode),
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
            &ICON_UV,
            uv_active,
            Tip::new(keys::ui_toolbar::UV_CHECKER)
                .describe(keys::ui_toolbar::UV_CHECKER_DESCRIPTION)
                .with_options()
                .page(Page::PanelsUvChecker),
        );
        if uv.clicked() {
            state.debug.active_material = ActiveMaterial::UvChecker;
        }
        if uv.secondary_clicked() {
            state.panels_open.toggle(OptionPanel::UvChecker);
        }

        let vertex_colors = icon_toggle_button_with_options(
            ui,
            &ICON_VERTEX_COLORS,
            vertex_colors_active,
            Tip::new(keys::ui_toolbar::VERTEX_COLORS)
                .describe(keys::ui_toolbar::VERTEX_COLORS_DESCRIPTION)
                .with_options()
                .page(Page::PanelsVertexColors),
        );
        if vertex_colors.clicked() {
            state.debug.active_material = ActiveMaterial::VertexColors;
        }
        if vertex_colors.secondary_clicked() {
            state.panels_open.toggle(OptionPanel::VertexColors);
        }

        let buffers = icon_toggle_button_with_options(
            ui,
            &ICON_BUFFERS,
            buffers_active,
            Tip::new(keys::ui_toolbar::BUFFERS)
                .describe(keys::ui_toolbar::BUFFERS_DESCRIPTION)
                .cycles()
                .with_options()
                .page(Page::PanelsBuffers),
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
                &ICON_SKIN_WEIGHTS,
                weights_active,
                Tip::new(keys::ui_toolbar::SKIN_WEIGHTS)
                    .describe(keys::ui_toolbar::SKIN_WEIGHTS_DESCRIPTION)
                    .page(Page::PanelsMaterialMode),
            )
            .clicked()
            {
                state.debug.active_material = ActiveMaterial::SkinWeights;
            }
        }
    });
}

/// Geometry-debug group: the face- and vertex-normal line overlays and the UV-seam
/// edges. Independent toggles — any combination can be active — and each retains its
/// own options panel.
fn draw_geometry_debug_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        option_toggle(
            ui,
            &ICON_NORMALS_FACE,
            &mut state.debug.face_normals,
            &mut state.panels_open,
            OptionPanel::FaceNormals,
            Tip::new(keys::ui_toolbar::FACE_NORMALS)
                .describe(keys::ui_toolbar::FACE_NORMALS_DESCRIPTION)
                .with_options()
                .page(Page::PanelsFaceNormals),
        );
        option_toggle(
            ui,
            &ICON_NORMALS_VERTEX,
            &mut state.debug.vertex_normals,
            &mut state.panels_open,
            OptionPanel::VertexNormals,
            Tip::new(keys::ui_toolbar::VERTEX_NORMALS)
                .describe(keys::ui_toolbar::VERTEX_NORMALS_DESCRIPTION)
                .with_options()
                .page(Page::PanelsVertexNormals),
        );
        option_toggle(
            ui,
            &ICON_UV_SEAM,
            &mut state.debug.uv_seams,
            &mut state.panels_open,
            OptionPanel::UvSeams,
            Tip::new(keys::ui_toolbar::UV_SEAMS)
                .describe(keys::ui_toolbar::UV_SEAMS_DESCRIPTION)
                .with_options()
                .page(Page::PanelsUvSeams),
        );
    });
}

/// View group: the bounding box (with its options panel), the object pivot marker,
/// the axis gizmo, and the floor grid. The bounding box and pivot draw in the 3D
/// scene; the gizmo and grid are independent display toggles.
fn draw_view_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        option_toggle(
            ui,
            &ICON_BBOX,
            &mut state.debug.show_bounding_box,
            &mut state.panels_open,
            OptionPanel::BoundingBox,
            Tip::new(keys::ui_toolbar::BOUNDING_BOX)
                .describe(keys::ui_toolbar::BOUNDING_BOX_DESCRIPTION)
                .with_options()
                .page(Page::PanelsBoundingBox),
        );

        // Skeleton overlay — hidden entirely for a model with no bones, rather
        // than shown disabled: an unrigged mesh has nothing to say about it.
        if state.has_bones {
            option_toggle(
                ui,
                &ICON_NODE_BONE,
                &mut state.debug.show_skeleton,
                &mut state.panels_open,
                OptionPanel::Skeleton,
                Tip::new(keys::ui_toolbar::SKELETON)
                    .describe(keys::ui_toolbar::SKELETON_DESCRIPTION)
                    .with_options()
                    .page(Page::PanelsSkeleton),
            );
        }

        // Pivot marker — a plain on/off toggle (no options), sitting between the
        // bounding box and the axis gizmo.
        icon_toggle_button(
            ui,
            &ICON_PIVOT,
            state.debug.show_pivot,
            Tip::new(keys::ui_toolbar::PIVOT)
                .describe(keys::ui_toolbar::PIVOT_DESCRIPTION)
                .page(Page::Overlays),
        )
        .clicked()
        .then(|| state.debug.show_pivot = !state.debug.show_pivot);

        icon_toggle_button(
            ui,
            &ICON_AXIS_GIZMO,
            state.show_axis_gizmo,
            Tip::new(keys::ui_toolbar::AXIS_GIZMO)
                .describe(keys::ui_toolbar::AXIS_GIZMO_DESCRIPTION)
                .page(Page::Viewport),
        )
        .clicked()
        .then(|| state.show_axis_gizmo = !state.show_axis_gizmo);

        icon_toggle_button(
            ui,
            &ICON_GRID,
            state.show_grid,
            Tip::new(keys::ui_toolbar::GRID)
                .describe(keys::ui_toolbar::GRID_DESCRIPTION)
                .page(Page::Viewport),
        )
        .clicked()
        .then(|| state.show_grid = !state.show_grid);
    });
}

/// Side-panel toggle group (every workspace): one button opening the Outliner (left) and
/// the Inspector (right) together. They are two halves of one workflow — the
/// Outliner picks a row, the Inspector describes it — so they share a toggle, and
/// it shows highlighted while they're open.
fn draw_side_panels_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        if icon_toggle_button(
            ui,
            &ICON_OUTLINER,
            state.side_panels_open,
            Tip::new(keys::ui_toolbar::SIDE_PANELS)
                .describe(keys::ui_toolbar::SIDE_PANELS_DESCRIPTION)
                .page(Page::OutlinerInspector),
        )
        .clicked()
        {
            state.side_panels_open = !state.side_panels_open;
        }
    });
}

/// The viewport tool toggle: View (camera only) or Select (clicking picks).
///
/// Innermost of the right-hand cluster, nearest the centre, because it is the
/// one tile there that changes what the *mouse* does rather than what is drawn -
/// and it is reached often enough to want the shorter travel. `Q` toggles the
/// same thing.
fn draw_tool_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        if icon_toggle_button(
            ui,
            &ICON_SELECT,
            state.tool == ViewportTool::Select,
            Tip::new(keys::ui_toolbar::SELECT_TOOL)
                .describe(keys::ui_toolbar::select_tool_description(
                    review_localization::tr(keys::ui_toolbar::SELECT_TOOL_KEY).into_owned(),
                ))
                .page(Page::Selection),
        )
        .clicked()
        {
            state.tool = state.tool.toggled();
        }
        // Comments are left in the 3D workspace; Opt lays its two meshes out side
        // by side and has no one surface to pin to.
        ui.add_enabled_ui(state.mode == WorkspaceMode::ThreeD, |ui| {
            if icon_toggle_button(
                ui,
                &ICON_COMMENT,
                state.tool == ViewportTool::Comment,
                Tip::new(keys::ui_toolbar::COMMENT_TOOL)
                    .describe(keys::ui_toolbar::comment_tool_description(
                        review_localization::tr(keys::ui_toolbar::COMMENT_TOOL_KEY).into_owned(),
                    ))
                    .page(Page::Comments),
            )
            .clicked()
            {
                state.tool = state.tool.toggled_comment();
            }
        });
    });
}

/// Help, alone in a group at the far right of the bar.
///
/// Its own group rather than sharing the side panels' one: every other tile on
/// the bar changes what you are looking at, and Help changes nothing - it is the
/// way *out* of the viewer, so it reads better as the thing past the end of the
/// tools than as one of them. Being last also gives it a fixed home, which is
/// what a control you reach for when you are lost needs most.
///
/// Left-click only: `F1` and each panel's own `?` are the context-sensitive ways
/// in, so there is nothing for a right-click to open here.
fn draw_help_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        if icon_toggle_button(
            ui,
            &ICON_HELP,
            state.help.open,
            Tip::new(keys::ui_toolbar::HELP)
                .describe(keys::ui_toolbar::HELP_DESCRIPTION)
                .page(Page::Index),
        )
        .clicked()
        {
            if state.help.open {
                state.help.open = false;
            } else {
                state.help.open_contents();
            }
        }
    });
}

fn draw_projection_group(ui: &mut egui::Ui, state: &mut UiState, width: f32) {
    toolbar_group_shell(ui, width, |ui| {
        let (icon, tooltip) = match state.projection_mode {
            ViewProjectionMode::Perspective => (
                &ICON_VIEW_PERSPECTIVE,
                Tip::new(keys::ui_toolbar::PERSPECTIVE)
                    .describe(keys::ui_toolbar::PERSPECTIVE_DESCRIPTION)
                    .page(Page::Viewport),
            ),
            ViewProjectionMode::Orthographic => (
                &ICON_VIEW_ORTHO,
                Tip::new(keys::ui_toolbar::ORTHOGRAPHIC)
                    .describe(keys::ui_toolbar::ORTHOGRAPHIC_DESCRIPTION)
                    .page(Page::Viewport),
            ),
        };

        if icon_toggle_button(ui, icon, false, tooltip).clicked() {
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
fn draw_uv_set_picker(ui: &mut egui::Ui, state: &mut UiState) {
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
    let width = size::TOOLBAR_UV_DROPDOWN_WIDTH;
    let labels = state.uv_sets.clone();
    let selected = labels[state.uv_view_channel as usize].clone();
    compact_combo(ui, "uv_set_picker", width, selected, |ui| {
        for (channel, label) in labels.iter().enumerate() {
            ui.selectable_value(&mut state.uv_view_channel, channel as u32, label);
        }
    });
}

fn segmented_mode_control(ui: &mut egui::Ui, mode: &mut WorkspaceMode) {
    for workspace in [
        WorkspaceMode::ThreeD,
        WorkspaceMode::Uv,
        WorkspaceMode::Texture,
        WorkspaceMode::Opt,
    ] {
        mode_segment(ui, mode, workspace);
    }
}

fn mode_segment(ui: &mut egui::Ui, mode: &mut WorkspaceMode, value: WorkspaceMode) {
    let width = size::MODE_SEGMENT_WIDTH;
    let label = review_localization::tr(labels::workspace(value));
    if segment_button(ui, label.as_ref(), *mode == value, width).clicked() {
        *mode = value;
    }
}

/// Channel radio group (RGB / R / G / B / A) shown on the left of the toolbar in
/// Texture mode: selects which channel of the viewed texture the Tex viewport
/// displays. Exactly one is active. The `A` segment is shown only when the viewed
/// image actually carries an alpha channel; an opaque (RGB / greyscale) source
/// hides it, and the group shrinks to the remaining segments.
fn draw_texture_channel_group(ui: &mut egui::Ui, state: &mut UiState) {
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

    let segment_w = size::TEXTURE_CHANNEL_SEGMENT_WIDTH;
    let padding = size::TOOLBAR_GROUP_PADDING;
    let gap = size::TOOLBAR_ICON_GAP;
    let count = if has_alpha { 5.0 } else { 4.0 };
    // Size the shell to exactly the visible segments (matching its own
    // padding/gap), so dropping A tightens the group instead of leaving a gap.
    let width = padding * 2.0 + count * segment_w + (count - 1.0) * gap;
    toolbar_group_shell(ui, width, |ui| {
        for channel in TextureChannelView::ALL {
            if channel == TextureChannelView::A && !has_alpha {
                continue;
            }
            let selected = state.texture_view.channel == channel;
            let label = review_localization::tr(channel.label());
            if segment_button(ui, label.as_ref(), selected, segment_w).clicked() {
                state.texture_view.channel = channel;
            }
        }
    });
}
