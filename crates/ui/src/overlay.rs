//! Top-level overlay orchestration: assembles the viewport scene callback and
//! the egui chrome (toolbar, option panel, gizmo, stats, status bar) each frame,
//! and returns the [`UiOutput`] intents for `app` to apply.

use std::sync::Arc;

use review_model::ModelData;
use review_render::{OrbitCamera, SceneCallback, UvCamera};

use crate::state::{UiOutput, UiState, WorkspaceMode, sync_debug_state};
use crate::theme::{self, color, size};
use crate::{gizmo, help, panels, stats, status_bar, toolbar};

/// Paint the viewport scene behind the egui chrome: the 3D scene in 3D mode, the
/// 2D UV viewport in UV mode. Texture mode draws nothing (placeholder).
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
        WorkspaceMode::ThreeD => SceneCallback::new(
            camera,
            state.projection_mode.into(),
            output_format,
            model,
            model_revision,
            state.debug,
        ),
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

/// Draw the full egui overlay and return the intents emitted this frame.
pub fn draw_overlay(ctx: &egui::Context, state: &mut UiState, camera: OrbitCamera) -> UiOutput {
    theme::apply_visuals(ctx);
    sync_debug_state(state);
    let mut output = UiOutput::default();

    let toolbar_height = theme::px(ctx, size::TOOLBAR_HEIGHT);
    let status_bar_height = theme::px(ctx, size::STATUS_BAR_HEIGHT);

    toolbar::draw(ctx, state);

    // The option panels, axis gizmo and stats overlay are all 3D-scene chrome;
    // the UV / Texture workspaces keep a clean viewport (just the UV dropdown in
    // the toolbar), so they only draw in 3D mode.
    if state.mode == WorkspaceMode::ThreeD {
        draw_option_panel(ctx, state, toolbar_height, status_bar_height);

        if state.show_axis_gizmo {
            let gizmo_response = egui::Area::new(egui::Id::new("axis_gizmo"))
                .fade_in(false)
                .anchor(
                    egui::Align2::RIGHT_TOP,
                    egui::vec2(
                        -theme::px(ctx, size::GIZMO_INSET),
                        toolbar_height + theme::px(ctx, size::GIZMO_INSET),
                    ),
                )
                .show(ctx, |ui| {
                    gizmo::draw_axis_gizmo(ui, ctx, camera, state.projection_mode.into())
                });
            output.axis_gizmo_action = gizmo_response.inner;
        }

        draw_stats_overlay(ctx, state, status_bar_height);
    }

    status_bar::draw(ctx, state);

    // The startup cheat-sheet sits on top of all the chrome (drawn last). It
    // consumes pointer input (so the chrome beneath stays inert while it's up);
    // `app` owns dismissing it — on any key, a click, a file drop, or a model
    // load — and double-clicking it opens the file picker.
    help::draw_help_overlay(ctx, state);

    output
}

/// Paint a full-screen cover over the entire viewer — the 3D/UV viewport *and*
/// the egui chrome — at `opacity` (1 = fully hidden, 0 = fully revealed). `app`
/// drives `opacity` from 1 down to 0 over the launch animation so the viewer
/// dissolves in from the startup black instead of popping in at once. At
/// `opacity <= 0` it draws nothing, so the steady state pays for no extra shape.
///
/// Drawn above every other layer (including the help overlay, which sits in
/// `Foreground`) so the whole composed frame fades together.
pub fn draw_startup_fade(ctx: &egui::Context, opacity: f32) {
    if opacity <= 0.0 {
        return;
    }
    let rect = ctx.screen_rect();
    let cover = theme::with_opacity(color::STARTUP_COVER, opacity);
    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("startup_fade"));
    ctx.layer_painter(layer).rect_filled(rect, 0.0, cover);
}

fn draw_option_panel(
    ctx: &egui::Context,
    state: &mut UiState,
    toolbar_height: f32,
    status_bar_height: f32,
) {
    let Some(panel) = state.active_panel else {
        return;
    };
    let left_panel_width = theme::px(ctx, size::LEFT_PANEL_WIDTH);
    let default_pos = egui::pos2(theme::px(ctx, 30.0), toolbar_height + theme::px(ctx, 18.0));
    let panel_pos = state.panel_pos.unwrap_or(default_pos);

    let area_response = egui::Area::new(egui::Id::new("option_panel"))
        // Persistent overlay chrome, not a transient popup: disable egui's
        // default fade-in. For an always-present anchored area `visible_last_frame`
        // reads false every frame, so the fade never completes and egui calls
        // `request_repaint()` forever — a busy-loop that pins the GPU. (See
        // egui area.rs:558-568.)
        .fade_in(false)
        .current_pos(panel_pos)
        // We drive position ourselves via the header drag handle; let egui's own
        // area-move stay off so a body drag can't fight our positioning (that
        // tug-of-war is what made the panel flicker).
        .movable(false)
        .show(ctx, |ui| {
            ui.set_width(left_panel_width);
            panels::draw_active_panel(ui, state, panel)
        });
    let outcome = area_response.inner;
    let panel_size = area_response.response.rect.size();

    if outcome.toggle_collapse {
        state.panel_collapsed = !state.panel_collapsed;
    }
    if outcome.close {
        state.active_panel = None;
    }

    // Keep the panel inside the viewport: never let it slide under the top
    // toolbar or the bottom status bar (and not off the left/right edges).
    let screen = ctx.screen_rect();
    let desired = panel_pos + outcome.drag_delta;
    let min_x = screen.left();
    let min_y = screen.top() + toolbar_height;
    let max_x = (screen.right() - panel_size.x).max(min_x);
    let max_y = (screen.bottom() - status_bar_height - panel_size.y).max(min_y);
    state.panel_pos = Some(egui::pos2(
        desired.x.clamp(min_x, max_x),
        desired.y.clamp(min_y, max_y),
    ));
}

fn draw_stats_overlay(ctx: &egui::Context, state: &UiState, status_bar_height: f32) {
    if !state.show_stats {
        return;
    }
    egui::Area::new(egui::Id::new("stats_overlay"))
        .fade_in(false)
        .anchor(
            egui::Align2::LEFT_BOTTOM,
            egui::vec2(
                theme::px(ctx, size::STATS_OVERLAY_MARGIN),
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
