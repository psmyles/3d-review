//! Top-level overlay orchestration: assembles the viewport scene callback and
//! the egui chrome (toolbar, option panel, gizmo, stats, status bar) each frame,
//! and returns the [`UiOutput`] intents for `app` to apply.

use std::sync::Arc;

use review_model::{ModelData, SceneBvh};
use review_render::{MaterialState, OrbitCamera, SceneCallback, SelectionView, UvCamera};

use crate::state::{OptionPanel, UiOutput, UiState, WorkspaceMode, sync_debug_state};
use crate::theme::{self, color, size};
use crate::{dimensions, gizmo, help, panels, stats, status_bar, texture_view, toolbar};

/// Paint the viewport scene behind the egui chrome: the 3D scene in 3D mode, the
/// 2D UV viewport in UV mode. Texture mode paints no wgpu scene — its 2D image
/// viewer is drawn in egui by [`draw_overlay`] (see [`texture_view`]).
pub fn draw_viewport_scene(
    ctx: &egui::Context,
    state: &UiState,
    camera: OrbitCamera,
    uv_camera: UvCamera,
    model: Arc<ModelData>,
    model_revision: u64,
    output_format: egui_wgpu::wgpu::TextureFormat,
) {
    let callback = match state.mode {
        WorkspaceMode::ThreeD => {
            // The editable material values ride in from the app→UI snapshot; the
            // scene callback uploads them into the renderer's material table. The
            // states carry `Arc`-shared decoded textures, so cloning is a refcount
            // bump, not a pixel copy.
            let materials: Vec<MaterialState> = state
                .materials_snapshot
                .iter()
                .map(|snapshot| snapshot.state.clone())
                .collect();
            // The Outliner selection + solo flag + highlight color + flash fade ride
            // in so the scene pass can flash / isolate the selection (Phase 2). The
            // fade is driven by `app` (invariant 6) and runs 1→0 over the flash.
            let selection = SelectionView {
                selection: state.selection,
                solo: state.solo,
                highlight_color: theme::color32_to_rgba(color::SELECTION_OUTLINE),
                fade: state.selection_fade,
            };
            // The Outliner's hidden mesh nodes ride in so the scene pass can filter
            // them out of the viewport draw + GTAO (Phase 2).
            let hidden_meshes: Vec<u32> = state
                .hidden_meshes
                .iter()
                .map(|&index| index as u32)
                .collect();
            SceneCallback::new(
                camera,
                state.projection_mode.into(),
                output_format,
                model,
                model_revision,
                state.debug,
                state.anti_aliasing,
                state.environment,
                state.gtao,
                state.tonemap,
                &materials,
                state.material_revision,
                selection,
                &hidden_meshes,
            )
        }
        WorkspaceMode::Uv => SceneCallback::new_uv(
            output_format,
            model,
            model_revision,
            uv_camera,
            state.uv_view_channel,
            state.uv_shading_mode,
        ),
        WorkspaceMode::Texture => return,
    };

    let rect = ctx.input(|input| input.screen_rect());
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, callback));
}

/// Draw the full egui overlay and return the intents emitted this frame. `model`
/// is the shared scene geometry and `bvh` an acceleration structure over it, both
/// read only for the bounding-box dimension labels' occlusion test (invariant 1:
/// borrowed, never copied). `bvh` is `None` until `app` has built it for the
/// current model (lazily, the first time the labels need it).
pub fn draw_overlay(
    ctx: &egui::Context,
    state: &mut UiState,
    camera: OrbitCamera,
    model: &ModelData,
    bvh: Option<&SceneBvh>,
    output_format: egui_wgpu::wgpu::TextureFormat,
) -> UiOutput {
    // Visuals + fonts are installed once at startup (`theme::init_style`); the
    // style is derived only from constant tokens, so there is nothing to re-apply
    // here each frame.
    sync_debug_state(state);
    let mut output = UiOutput::default();

    let toolbar_height = theme::px(ctx, size::TOOLBAR_HEIGHT);
    let status_bar_height = theme::px(ctx, size::STATUS_BAR_HEIGHT);

    // Native chrome panels first: the top toolbar and bottom status bar carve their
    // bands, then the dockable side panels fill the middle — declared in this order
    // so the side panels sit *between* the bars, not under them.
    toolbar::draw(ctx, state);
    status_bar::draw(ctx, state);

    // The side panels, option panels, axis gizmo and stats overlay are all 3D-scene
    // chrome; the UV / Texture workspaces keep a clean viewport (just the UV dropdown
    // in the toolbar), so they only draw in 3D mode.
    if state.mode == WorkspaceMode::ThreeD {
        // Outliner (left) + Inspector (right) dockable side panels. They paint over
        // the full-window background scene (exactly as the toolbar / status bar
        // already do); their live widths inset the floating viewport chrome below so
        // the gizmo / stats never land on top of a panel. The Inspector emits
        // material-edit intents for `app` to apply (invariant 2).
        let side = draw_side_panels(ctx, state, model);
        output.material_edit = side.inspector.material_edit;
        output.texture = side.inspector.texture;

        // The free viewport: the screen minus the chrome bands (toolbar top,
        // status bar bottom) and the open side panels (left/right). Floating
        // chrome — the dimension labels and the option windows — is kept inside
        // this rect so it never overlaps the toolbar icons or a side panel.
        let screen = ctx.screen_rect();
        let viewport = egui::Rect::from_min_max(
            egui::pos2(
                screen.left() + side.left_inset,
                screen.top() + toolbar_height,
            ),
            egui::pos2(
                screen.right() - side.right_inset,
                screen.bottom() - status_bar_height,
            ),
        );

        // Bounding-box dimension labels sit on the viewport (under the chrome).
        // The measured box is resolved here (cached for the "visible only" scan)
        // so the overlay never redoes the O(triangle) bounds walk per frame.
        let bounds = state.measured_bounds(model);
        dimensions::draw_dimension_labels(ctx, state, camera, model, bvh, bounds, viewport);

        draw_option_panels(ctx, state, viewport);

        if state.show_axis_gizmo {
            let gizmo_response = egui::Area::new(egui::Id::new("axis_gizmo"))
                .fade_in(false)
                .anchor(
                    egui::Align2::RIGHT_TOP,
                    egui::vec2(
                        -(theme::px(ctx, size::GIZMO_INSET) + side.right_inset),
                        toolbar_height + theme::px(ctx, size::GIZMO_INSET),
                    ),
                )
                .show(ctx, |ui| {
                    gizmo::draw_axis_gizmo(ui, ctx, camera, state.projection_mode.into())
                });
            output.axis_gizmo_action = gizmo_response.inner;
        }

        draw_stats_overlay(ctx, state, status_bar_height, side.left_inset);
    } else if state.mode == WorkspaceMode::Texture {
        // The Tex workspace paints a 2D image viewer (channel-isolated, pan/zoom)
        // over a chosen background fill, plus its own floating stats panel. The
        // image itself is drawn by a wgpu paint callback, which needs egui's
        // framebuffer format to build its pipeline.
        texture_view::draw(ctx, state, output_format);
    }

    // The startup cheat-sheet sits on top of all the chrome (drawn last). It
    // consumes pointer input (so the chrome beneath stays inert while it's up);
    // `app` owns dismissing it — on any key, a click, a file drop, or a model
    // load — and double-clicking it opens the file picker.
    help::draw_help_overlay(ctx, state);

    output
}

/// Draw every open tool option panel as its own native `egui::Window`
/// (resizable, collapsible, closable, drop-shadowed — egui owns each window's
/// position/size/collapsed state in memory, constrained to `viewport` so they
/// stay inside the free scene area, and several can be open at once). A window's
/// title-bar X clears it from [`UiState::panels_open`].
fn draw_option_panels(ctx: &egui::Context, state: &mut UiState, viewport: egui::Rect) {
    for (slot, panel) in OptionPanel::ALL.into_iter().enumerate() {
        if !state.panels_open.is_open(panel) {
            continue;
        }
        // Cascade fresh windows down-right from the viewport's top-left so several
        // opened at once don't land exactly atop each other. egui only honors this
        // the first time a given window id appears; afterwards the user's dragged
        // position (kept in egui memory) wins.
        let step = size::PANEL_CASCADE_STEP * slot as f32;
        let margin = theme::px(ctx, size::OVERLAY_MARGIN);
        let default_pos = egui::pos2(
            viewport.left() + margin + step,
            viewport.top() + margin + step,
        );
        // egui's `.open(&mut bool)` paints the title-bar X and flips this false
        // when it's clicked; mirror that back into the open-set after the window.
        let mut open = true;
        egui::Window::new(panel.title())
            .id(egui::Id::new(panel.window_id()))
            .open(&mut open)
            // The option panels have compact, fixed content (a two-column table),
            // so they aren't resizable — which also drops egui's bottom-right
            // resize grip. The body pins a consistent width (see `draw_panel_body`).
            .resizable(false)
            .collapsible(true)
            .default_pos(default_pos)
            // Keep the window inside the free viewport so it can never be dragged
            // over the toolbar, the status bar or a side panel.
            .constrain_to(viewport)
            // Persistent chrome, not a transient popup: skip egui's fade so an
            // always-present window never spins the on-demand redraw loop
            // (invariant 6).
            .fade_in(false)
            .fade_out(false)
            .show(ctx, |ui| panels::draw_panel_body(ui, state, panel));
        if !open {
            state.panels_open.set(panel, false);
        }
    }
}

/// The result of laying out the dockable side panels: the Inspector's emitted
/// intents (material edit / texture import / assign / clear / remove) plus the
/// live widths of the open panels, used to inset the floating viewport chrome
/// (gizmo / stats) so it doesn't land over a panel.
struct SidePanelLayout {
    inspector: panels::inspector::InspectorOutput,
    left_inset: f32,
    right_inset: f32,
}

/// Draw the dockable Outliner (left) and Inspector (right) side panels and return
/// the Inspector's material edit plus the panels' live widths. Both are native
/// `egui::SidePanel`s — resizable by dragging their inner edge, with egui owning
/// the width across frames. The Outliner mutates [`UiState::selection`] directly;
/// the Inspector returns an intent for `app` to apply (invariant 2).
fn draw_side_panels(
    ctx: &egui::Context,
    state: &mut UiState,
    model: &ModelData,
) -> SidePanelLayout {
    let mut left_inset = 0.0;
    if state.outliner_open {
        let response = egui::SidePanel::left("outliner_panel")
            .resizable(true)
            .default_width(size::SIDE_PANEL_DEFAULT_WIDTH)
            .width_range(size::SIDE_PANEL_MIN_WIDTH..=size::OUTLINER_MAX_WIDTH)
            .show(ctx, |ui| panels::outliner::body(ui, state, model));
        left_inset = response.response.rect.width();
    }

    let mut right_inset = 0.0;
    let mut inspector = panels::inspector::InspectorOutput::default();
    if state.inspector_open {
        let response = egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(size::SIDE_PANEL_DEFAULT_WIDTH)
            .width_range(size::SIDE_PANEL_MIN_WIDTH..=size::OUTLINER_MAX_WIDTH)
            .show(ctx, |ui| panels::inspector::body(ui, state, model));
        right_inset = response.response.rect.width();
        inspector = response.inner;
    }

    SidePanelLayout {
        inspector,
        left_inset,
        right_inset,
    }
}

fn draw_stats_overlay(
    ctx: &egui::Context,
    state: &UiState,
    status_bar_height: f32,
    left_inset: f32,
) {
    if !state.show_stats {
        return;
    }
    egui::Area::new(egui::Id::new("stats_overlay"))
        .fade_in(false)
        .anchor(
            egui::Align2::LEFT_BOTTOM,
            egui::vec2(
                left_inset + theme::px(ctx, size::STATS_OVERLAY_MARGIN),
                -(status_bar_height + theme::px(ctx, size::STATS_OVERLAY_MARGIN)),
            ),
        )
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
                    ui.set_width(size::STATS_PANEL_WIDTH);
                    stats::stats_grid(ui, state);
                });
        });
}
