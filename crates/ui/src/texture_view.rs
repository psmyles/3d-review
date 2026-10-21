//! The Tex viewport: a 2D image viewer for the scene texture pool.
//!
//! The viewed texture is drawn by the renderer into the same frame, behind the egui
//! chrome, not by a hand-rolled image blit. This module owns only the *interaction*:
//! it lays out the canvas, handles pan/zoom/fit, paints the background fill, and
//! emits the placement + channel selection that `app` feeds to the renderer. Channel isolation is a uniform the shader swizzles on,
//! so switching RGB/R/G/B/A is free
//! (no CPU rebuild, no re-upload); the GPU texture is uploaded once per image and
//! reused (invariant 2: the pixels live in the app-owned [`TexturePoolEntry`]; the
//! UI emits only plain placement + channel values).

use std::sync::{Arc, Weak};

use review_render::DecodedImage;

use crate::keys;
use crate::state::{
    TexViewRequest, TexViewTransition, TexturePoolEntry, TextureViewState, UiState,
};
use crate::theme::{color, font, motion, size};

/// Draw the Tex viewport: the central image canvas behind the chrome, in the
/// space the side panels leave (the viewed texture's measured properties are in
/// the Inspector). Called from the overlay only in
/// [`crate::state::WorkspaceMode::Texture`], after the side panels have taken
/// their share of the window.
pub(crate) fn draw(root: &mut egui::Ui, state: &mut UiState) {
    let ctx = &root.ctx().clone();
    // Keep the selection in range (a removed texture may have shrunk the pool).
    if state.texture_view.selected >= state.texture_pool.len() {
        state.texture_view.selected = 0;
    }
    // Clone the viewed entry (an `Arc` refcount bump, not a pixel copy) so the
    // canvas closure can borrow `state.texture_view` mutably without colliding
    // with the `state.texture_pool` borrow.
    let entry = state.texture_pool.get(state.texture_view.selected).cloned();

    // No frame fill: the background fill + the image are drawn by `app` through the
    // renderer, behind this transparent CentralPanel. The canvas rect is captured
    // here and handed to `app` to place the image.
    let canvas_rect = egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(root, |ui| {
            let rect = ui.max_rect();
            match &entry {
                Some(entry) => draw_canvas(ui, ctx, &mut state.texture_view, rect, entry),
                None => draw_empty_hint(ui, rect),
            }
            rect
        })
        .inner;
    state.texture_canvas = Some(canvas_rect);
}

/// Handle the canvas interaction (pan / zoom / fit). The background fill is the
/// frame's clear colour and the image is a renderer draw; this module owns only
/// placement + interaction.
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
    //
    // The identity is the pointer `view.fitted_key` records *and* a `Weak` to the
    // image kept beside the canvas. The pointer alone is a number: once the
    // fitted image was freed, the next one decoded could be allocated at the same
    // address and read as already fitted. A `Weak` pins the allocation it points
    // at, so while it is held no other image can arrive there.
    let fitted_id = ui.id().with("tex_canvas_fitted");
    let identity = Arc::as_ptr(image) as usize;
    let fitted = view.fitted_key == Some(identity)
        && ui
            .data(|data| data.get_temp::<Weak<DecodedImage>>(fitted_id))
            .is_some_and(|fitted| std::ptr::eq(fitted.as_ptr(), Arc::as_ptr(image)));
    if !fitted || response.double_clicked() {
        // A canvas with no area — a window minimized, or squeezed shut between
        // the side panels — has nothing to fit to. The image stays unfitted
        // rather than being marked as fitted at a placeholder zoom, so the first
        // frame with room fits it.
        if let Some((zoom, pan)) = fit_target(rect, img_px) {
            view.zoom = zoom;
            view.pan = pan;
            view.fitted_key = Some(identity);
            ui.data_mut(|data| data.insert_temp(fitted_id, Arc::downgrade(image)));
        }
        view.transition = None;
        view.request = None;
    }

    // A pending animated request — the zoom-readout toggle (100% ↔ fit) or the
    // `F` / `R` frame reset — resolved here (where the viewport rect is known) into
    // a concrete pan/zoom target, then eased over `TEXTURE_ZOOM_ANIM_SECS`. A fit
    // asked for while the canvas has no area waits for one.
    if let Some(request) = view.request {
        let target = match request {
            TexViewRequest::Zoom(zoom) => Some((zoom, view.pan)),
            TexViewRequest::Fit => fit_target(rect, img_px),
        };
        if let Some((to_zoom, to_pan)) = target {
            view.request = None;
            start_transition(view, ctx, to_zoom, to_pan);
        }
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
                dy * motion::TEXTURE_DRAG_ZOOM_SPEED,
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
                scroll * motion::TEXTURE_ZOOM_SPEED,
            );
        }
    }

    // Advance any in-flight ease toward its target, requesting a repaint until it
    // settles (the on-demand redraw loop honours egui's repaint request).
    advance_transition(view, ctx);

    // The image itself is drawn by `app` through `review_render`'s `TexGpu`
    // (channel isolation is a shader-uniform swizzle there); this module owns
    // only placement + interaction (pan/zoom/fit) and the background fill.
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
/// with a small margin. `None` for a viewport with no area, which there is no
/// fitting anything to.
fn fit_target(rect: egui::Rect, img_px: egui::Vec2) -> Option<(f32, egui::Vec2)> {
    let avail = rect.size() * size::TEXTURE_FIT_MARGIN;
    let scale = (avail.x / img_px.x).min(avail.y / img_px.y);
    (scale.is_finite() && scale > 0.0).then(|| {
        (
            scale.clamp(size::TEXTURE_ZOOM_MIN, size::TEXTURE_ZOOM_MAX),
            egui::Vec2::ZERO,
        )
    })
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
    let t = if motion::TEXTURE_ZOOM_ANIM_SECS > 0.0 {
        (elapsed / motion::TEXTURE_ZOOM_ANIM_SECS).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let eased = review_render::ease_in_out_cubic(t);
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

/// The centered hint shown when the texture pool is empty.
fn draw_empty_hint(ui: &mut egui::Ui, rect: egui::Rect) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        review_localization::tr(keys::ui_texture_view::EMPTY_HINT),
        egui::FontId::proportional(font::VIEWPORT_EMPTY_HINT),
        color::TEXT_MUTED,
    );
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(name: &str) -> TexturePoolEntry {
        let (width, height) = (512, 256);
        TexturePoolEntry {
            path: PathBuf::from(name),
            image: Arc::new(DecodedImage {
                width,
                height,
                rgba: vec![255; (width * height * 4) as usize],
                source_channels: 4,
                source_bit_depth: 8,
            }),
            file_size: 0,
        }
    }

    /// One pass of the canvas over `rect`, in a headless context.
    fn draw_over(
        ctx: &egui::Context,
        view: &mut TextureViewState,
        rect: egui::Rect,
        entry: &TexturePoolEntry,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            draw_canvas(ui, &ctx, view, rect, entry);
        });
        output.drop_without_applying_deltas();
    }

    fn canvas(width: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(width, 600.0))
    }

    fn fitted_zoom(rect: egui::Rect, entry: &TexturePoolEntry) -> f32 {
        let img_px = egui::vec2(entry.image.width as f32, entry.image.height as f32);
        fit_target(rect, img_px).expect("a canvas with area").0
    }

    /// The Tex workspace can be entered with the side panels squeezing the canvas
    /// shut. That fit used to fall back to 1:1 and still be recorded as done, so
    /// the image stayed at 1:1 once the canvas opened up.
    #[test]
    fn a_fit_on_a_canvas_with_no_area_waits_for_one() {
        let ctx = egui::Context::default();
        let mut view = TextureViewState::default();
        let entry = entry("albedo.png");

        draw_over(&ctx, &mut view, canvas(0.0), &entry);
        assert_eq!(
            view.fitted_key, None,
            "a fit to no area was recorded as done"
        );

        draw_over(&ctx, &mut view, canvas(800.0), &entry);
        assert_eq!(view.zoom, fitted_zoom(canvas(800.0), &entry));
        assert!(view.fitted_key.is_some());
    }

    /// Once fitted, the image keeps the view the user gives it; a different image
    /// is fitted afresh.
    #[test]
    fn only_a_new_image_is_refitted() {
        let ctx = egui::Context::default();
        let mut view = TextureViewState::default();
        let first = entry("albedo.png");

        draw_over(&ctx, &mut view, canvas(800.0), &first);
        view.zoom = 3.0;
        draw_over(&ctx, &mut view, canvas(800.0), &first);
        assert_eq!(view.zoom, 3.0, "the image already on screen was refitted");

        drop(first);
        let reloaded = entry("albedo.png");
        draw_over(&ctx, &mut view, canvas(800.0), &reloaded);
        assert_eq!(
            view.zoom,
            fitted_zoom(canvas(800.0), &reloaded),
            "a reloaded image kept the old one's view"
        );
    }
}
