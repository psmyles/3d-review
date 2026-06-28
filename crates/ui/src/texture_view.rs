//! The Tex viewport: a 2D image viewer for the scene texture pool.
//!
//! The viewed texture is drawn by `review_render`'s `TexGpu` (a Direct3D 11 draw
//! issued by `app`, behind the egui chrome), not a hand-rolled image blit. This
//! module owns only the *interaction*: it lays out the canvas, handles pan/zoom/fit,
//! paints the background fill, and emits the placement + channel selection that
//! `app` feeds to `TexGpu`. Channel isolation is a uniform the shader swizzles on,
//! so switching RGB/R/G/B/A is free
//! (no CPU rebuild, no re-upload); the GPU texture is uploaded once per image and
//! reused (invariant 2: the pixels live in the app-owned [`TexturePoolEntry`]; the
//! UI emits only plain placement + channel values).

use std::sync::Arc;

use crate::state::{
    TexViewRequest, TexViewTransition, TexturePoolEntry, TextureViewState, UiState,
};
use crate::stats;
use crate::theme::{self, color, font, size};

/// Draw the Tex viewport: the central image canvas behind the chrome, plus the
/// floating texture-stats panel when toggled on. Called from the overlay only in
/// [`crate::state::WorkspaceMode::Texture`]. `output_format` is egui's framebuffer
/// format, needed to build the image paint callback's pipeline.
pub(crate) fn draw(ctx: &egui::Context, state: &mut UiState) {
    // Keep the selection in range (a removed texture may have shrunk the pool).
    if state.texture_view.selected >= state.texture_pool.len() {
        state.texture_view.selected = 0;
    }
    // Clone the viewed entry (an `Arc` refcount bump, not a pixel copy) so the
    // canvas closure can borrow `state.texture_view` mutably without colliding
    // with the `state.texture_pool` borrow.
    let entry = state.texture_pool.get(state.texture_view.selected).cloned();

    // No frame fill: the background fill + the image are drawn by `app` through the
    // D3D11 RHI (migration Phase 4), behind this transparent CentralPanel. The canvas
    // rect is captured here and handed to `app` to place the image.
    let canvas_rect = egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            match &entry {
                Some(entry) => draw_canvas(ui, ctx, &mut state.texture_view, rect, entry),
                None => draw_empty_hint(ui, ctx, rect),
            }
            rect
        })
        .inner;
    state.texture_canvas = Some(canvas_rect);

    if let (Some(entry), true) = (&entry, state.texture_view.show_stats) {
        draw_texture_stats_overlay(ctx, entry);
    }
}

/// Handle the canvas interaction (pan / zoom / fit). The background fill + the image
/// itself are drawn by `app` through the D3D11 RHI (migration Phase 4); this module
/// owns only placement + interaction.
fn draw_canvas(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    view: &mut TextureViewState,
    rect: egui::Rect,
    entry: &TexturePoolEntry,
) {
    let image = &entry.image;
    let img_px = egui::vec2(image.width.max(1) as f32, image.height.max(1) as f32);

    // The whole canvas is the interaction surface: drag pans, wheel zooms,
    // double-click re-fits. `click_and_drag` lets egui mark the pointer consumed
    // so `app` doesn't also orbit the (hidden) 3D camera.
    let response = ui.interact(
        rect,
        ui.id().with("tex_canvas"),
        egui::Sense::click_and_drag(),
    );

    // Fit the image to the viewport instantly on first show or a texture switch /
    // disk reload (the decoded-image `Arc` identity changes), and on a
    // double-click — a new image should simply appear fitted, with no animation.
    // Both cancel any in-flight ease / pending request.
    let identity = Arc::as_ptr(image) as usize;
    if view.fitted_key != Some(identity) || response.double_clicked() {
        let (zoom, pan) = fit_target(rect, img_px);
        view.zoom = zoom;
        view.pan = pan;
        view.fitted_key = Some(identity);
        view.transition = None;
        view.request = None;
    }

    // A pending animated request — the zoom-readout toggle (100% ↔ fit) or the
    // `F` / `R` frame reset — resolved here (where the viewport rect is known) into
    // a concrete pan/zoom target, then eased over `TEXTURE_ZOOM_ANIM_SECS`.
    if let Some(request) = view.request.take() {
        let (to_zoom, to_pan) = match request {
            TexViewRequest::Zoom(zoom) => (zoom, view.pan),
            TexViewRequest::Fit => fit_target(rect, img_px),
        };
        start_transition(view, ctx, to_zoom, to_pan);
    }

    // Left-drag pans. Right-drag zooms (drag down = zoom in), anchored on the
    // viewport center to mirror the UV viewport's right-drag zoom. Any direct
    // interaction cancels an in-flight ease so the user takes over immediately.
    if response.dragged_by(egui::PointerButton::Primary) {
        view.transition = None;
        view.pan += response.drag_delta();
    } else if response.dragged_by(egui::PointerButton::Secondary) {
        let dy = response.drag_delta().y;
        if dy != 0.0 {
            view.transition = None;
            zoom_at(
                view,
                rect.center(),
                rect.center(),
                dy * size::TEXTURE_DRAG_ZOOM_SPEED,
            );
        }
    }

    // Wheel zoom, anchored at the cursor so the texel under the pointer stays put.
    if let Some(pointer) = response.hover_pos() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            view.transition = None;
            zoom_at(
                view,
                rect.center(),
                pointer,
                scroll * size::TEXTURE_ZOOM_SPEED,
            );
        }
    }

    // Advance any in-flight ease toward its target, requesting a repaint until it
    // settles (the on-demand redraw loop honours egui's repaint request).
    advance_transition(view, ctx);

    // DORMANT (D3D11 migration): the image itself is drawn by `app` through the
    // D3D11 RHI (migration Phase 4); channel isolation is a shader-uniform swizzle
    // there. This module owns only placement + interaction (pan/zoom/fit) and the
    // background fill, so for now the viewport shows the chosen background only.
}

/// Apply an exponential zoom step (`exponent` measured in zoom e-folds) about a
/// screen `anchor`, keeping the image texel under that anchor fixed while the
/// `center`-relative pan is rescaled. The wheel anchors on the cursor; the
/// right-drag zoom anchors on the viewport center.
fn zoom_at(view: &mut TextureViewState, center: egui::Pos2, anchor: egui::Pos2, exponent: f32) {
    let old = view.zoom;
    let new = (old * exponent.exp()).clamp(size::TEXTURE_ZOOM_MIN, size::TEXTURE_ZOOM_MAX);
    let factor = new / old;
    let to_anchor = anchor - (center + view.pan);
    view.pan += to_anchor * (1.0 - factor);
    view.zoom = new;
}

/// The (zoom, pan) that fits the image to the viewport — centered (zero pan),
/// with a small margin. A degenerate viewport falls back to 1:1.
fn fit_target(rect: egui::Rect, img_px: egui::Vec2) -> (f32, egui::Vec2) {
    let avail = rect.size() * size::TEXTURE_FIT_MARGIN;
    let scale = (avail.x / img_px.x).min(avail.y / img_px.y);
    let zoom = if scale.is_finite() && scale > 0.0 {
        scale.clamp(size::TEXTURE_ZOOM_MIN, size::TEXTURE_ZOOM_MAX)
    } else {
        1.0
    };
    (zoom, egui::Vec2::ZERO)
}

/// Begin a [`TexViewTransition`] easing the current pan/zoom toward
/// `(to_zoom, to_pan)`. A no-op target (already there) is snapped without
/// scheduling an animation, so the readout toggle never spins the redraw loop for
/// nothing.
fn start_transition(
    view: &mut TextureViewState,
    ctx: &egui::Context,
    to_zoom: f32,
    to_pan: egui::Vec2,
) {
    if (to_zoom - view.zoom).abs() <= f32::EPSILON && (to_pan - view.pan).length() <= f32::EPSILON {
        view.zoom = to_zoom;
        view.pan = to_pan;
        view.transition = None;
        return;
    }
    view.transition = Some(TexViewTransition {
        from_zoom: view.zoom,
        to_zoom,
        from_pan: view.pan,
        to_pan,
        start_time: ctx.input(|input| input.time),
    });
    ctx.request_repaint();
}

/// Advance an in-flight pan/zoom ease, writing the eased values into `view` and
/// requesting another frame until it completes (then snapping exactly to the
/// target and clearing the transition).
fn advance_transition(view: &mut TextureViewState, ctx: &egui::Context) {
    let Some(transition) = view.transition else {
        return;
    };
    let now = ctx.input(|input| input.time);
    let elapsed = (now - transition.start_time) as f32;
    let t = if size::TEXTURE_ZOOM_ANIM_SECS > 0.0 {
        (elapsed / size::TEXTURE_ZOOM_ANIM_SECS).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let eased = ease_in_out_cubic(t);
    view.zoom = transition.from_zoom + (transition.to_zoom - transition.from_zoom) * eased;
    view.pan = transition.from_pan + (transition.to_pan - transition.from_pan) * eased;
    if t >= 1.0 {
        view.zoom = transition.to_zoom;
        view.pan = transition.to_pan;
        view.transition = None;
    } else {
        ctx.request_repaint();
    }
}

/// Ease-in-out cubic on `t ∈ [0, 1]` — the same shape the 3D camera transition
/// uses, so the Tex view's snaps read like the rest of the app.
fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// The centered hint shown when the texture pool is empty.
fn draw_empty_hint(ui: &mut egui::Ui, ctx: &egui::Context, rect: egui::Rect) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "No textures loaded - drop image files here or add them in the Inspector",
        egui::FontId::proportional(theme::px(ctx, font::VIEWPORT_EMPTY_HINT)),
        color::TEXT_MUTED,
    );
}

/// The floating texture-stats panel, anchored bottom-left above the status bar —
/// the Tex viewport's analogue of the model-stats overlay (matching frame + inset).
fn draw_texture_stats_overlay(ctx: &egui::Context, entry: &TexturePoolEntry) {
    let status_bar_height = theme::px(ctx, size::STATUS_BAR_HEIGHT);
    egui::Area::new(egui::Id::new("texture_stats_overlay"))
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
                    ui.set_width(size::TEXTURE_STATS_PANEL_WIDTH);
                    stats::texture_stats_grid(ui, entry);
                });
        });
}
