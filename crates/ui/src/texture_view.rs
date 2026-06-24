//! The Tex viewport: a 2D image viewer for the scene texture pool.
//!
//! Unlike the 3D / UV viewports (which paint through the wgpu scene callback),
//! the Tex view is pure egui: the selected pooled texture is uploaded as an egui
//! texture (with the chosen channel isolated on the CPU) and painted with
//! interactive pan/zoom over a chosen background fill. All pixels come from the
//! app-owned [`TexturePoolEntry`] snapshots (invariant 2) — this module owns no
//! renderer/model state, only the [`TextureViewState`] view parameters.
//!
//! The toolbar's channel group + the status bar's background group + the texture
//! picker drive the parameters; this module renders the central canvas and the
//! floating stats panel.

use std::sync::Arc;

use review_render::DecodedImage;

use crate::state::{TextureChannelView, TexturePoolEntry, TextureViewState, UiState};
use crate::stats;
use crate::theme::{self, color, font, size};

/// Draw the Tex viewport: the central image canvas behind the chrome, plus the
/// floating texture-stats panel when toggled on. Called from the overlay only in
/// [`crate::state::WorkspaceMode::Texture`].
pub(crate) fn draw(ctx: &egui::Context, state: &mut UiState) {
    // Keep the selection in range (a removed texture may have shrunk the pool).
    if state.texture_view.selected >= state.texture_pool.len() {
        state.texture_view.selected = 0;
    }
    // Clone the viewed entry (an `Arc` refcount bump, not a pixel copy) so the
    // canvas closure can borrow `state.texture_view` mutably without colliding
    // with the `state.texture_pool` borrow.
    let entry = state.texture_pool.get(state.texture_view.selected).cloned();

    egui::CentralPanel::default()
        // No frame fill: this module paints its own background (the chosen fill),
        // and the wgpu clear shows through the transparent margins.
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            match &entry {
                Some(entry) => draw_canvas(ui, ctx, &mut state.texture_view, rect, entry),
                None => draw_empty_hint(ui, ctx, rect),
            }
        });

    if let (Some(entry), true) = (&entry, state.texture_view.show_stats) {
        draw_texture_stats_overlay(ctx, entry);
    }
}

/// Paint the background fill, then the viewed texture with live pan/zoom and the
/// selected channel isolated.
fn draw_canvas(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    view: &mut TextureViewState,
    rect: egui::Rect,
    entry: &TexturePoolEntry,
) {
    paint_background(ui, ctx, rect, view.background);

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

    // Fit the image to the viewport on first show, on a texture switch / disk
    // reload (the decoded-image `Arc` identity changes), or on a double-click.
    let identity = Arc::as_ptr(image) as usize;
    if view.fitted_key != Some(identity) || response.double_clicked() {
        fit(view, rect, img_px);
        view.fitted_key = Some(identity);
    }

    if response.dragged() {
        view.pan += response.drag_delta();
    }

    // Wheel zoom, anchored at the cursor so the texel under the pointer stays put.
    if let Some(pointer) = response.hover_pos() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let old = view.zoom;
            let new = (old * (scroll * size::TEXTURE_ZOOM_SPEED).exp())
                .clamp(size::TEXTURE_ZOOM_MIN, size::TEXTURE_ZOOM_MAX);
            let factor = new / old;
            let to_pointer = pointer - (rect.center() + view.pan);
            view.pan += to_pointer * (1.0 - factor);
            view.zoom = new;
        }
    }

    let img_rect = egui::Rect::from_center_size(rect.center() + view.pan, img_px * view.zoom);
    if let Some(texture) = canvas_texture(ui, entry, view.channel, identity) {
        egui::Image::from_texture(egui::load::SizedTexture::from_handle(&texture))
            .paint_at(ui, img_rect);
    }
}

/// Reset the pan/zoom so the image fits the viewport (centered, with a small
/// margin). A degenerate viewport falls back to 1:1.
fn fit(view: &mut TextureViewState, rect: egui::Rect, img_px: egui::Vec2) {
    let avail = rect.size() * size::TEXTURE_FIT_MARGIN;
    let scale = (avail.x / img_px.x).min(avail.y / img_px.y);
    view.zoom = if scale.is_finite() && scale > 0.0 {
        scale.clamp(size::TEXTURE_ZOOM_MIN, size::TEXTURE_ZOOM_MAX)
    } else {
        1.0
    };
    view.pan = egui::Vec2::ZERO;
}

/// Paint the chosen background fill across the canvas. The checker is a tiled 2×2
/// texture (so transparency in the image reads against it); the solids are a
/// single filled rect.
fn paint_background(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    rect: egui::Rect,
    background: crate::state::TextureBackground,
) {
    use crate::state::TextureBackground::*;
    let painter = ui.painter().clone();
    match background {
        Black => {
            painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
        }
        White => {
            painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
        }
        Grey => {
            painter.rect_filled(rect, 0.0, color::TEXTURE_BG_GREY);
        }
        Checker => {
            let texture = checker_texture(ui);
            // The 2×2 texture spans one full checker period over two cells, so the
            // uv extent is the rect measured in periods.
            let period = (2.0 * theme::px(ctx, size::TEXTURE_CHECKER_CELL)).max(1.0);
            let uv = egui::Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(rect.width() / period, rect.height() / period),
            );
            painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
        }
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

/// Lazily build + cache the egui texture for the viewed image with the selected
/// channel isolated. Single-slot cache in egui temp data keyed by the image
/// identity + channel: switching texture or channel rebuilds (dropping the old
/// handle frees its GPU texture). Magnified nearest (crisp texels when zoomed in),
/// minified linear (smooth when fit).
fn canvas_texture(
    ui: &mut egui::Ui,
    entry: &TexturePoolEntry,
    channel: TextureChannelView,
    identity: usize,
) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("tex_canvas_texture");
    if let Some((cached_id, cached_channel, handle)) =
        ui.data(|data| data.get_temp::<(usize, TextureChannelView, egui::TextureHandle)>(id))
    {
        if cached_id == identity && cached_channel == channel {
            return Some(handle);
        }
    }

    let color_image = channel_color_image(&entry.image, channel)?;
    let options = egui::TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        ..egui::TextureOptions::LINEAR
    };
    let handle = ui.ctx().load_texture(
        format!("tex:{}", entry.path.display()),
        color_image,
        options,
    );
    ui.data_mut(|data| data.insert_temp(id, (identity, channel, handle.clone())));
    Some(handle)
}

/// Build the egui `ColorImage` for one channel view of a decoded image. `Rgb`
/// keeps the full color + alpha (transparency composites over the background); a
/// single channel is replicated across RGB as opaque greyscale.
fn channel_color_image(
    image: &DecodedImage,
    channel: TextureChannelView,
) -> Option<egui::ColorImage> {
    let width = image.width.max(1) as usize;
    let height = image.height.max(1) as usize;
    if image.rgba.len() < width * height * 4 {
        return None;
    }
    let size = [width, height];
    match channel.channel_offset() {
        None => Some(egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba)),
        Some(offset) => {
            let mut gray = Vec::with_capacity(width * height * 4);
            for pixel in image.rgba.chunks_exact(4) {
                let value = pixel[offset];
                gray.extend_from_slice(&[value, value, value, 255]);
            }
            Some(egui::ColorImage::from_rgba_unmultiplied(size, &gray))
        }
    }
}

/// Lazily build + cache the 2×2 checkerboard background texture (set to repeat).
/// Cached in egui temp data so the handle outlives the frame that paints with it.
fn checker_texture(ui: &mut egui::Ui) -> egui::TextureHandle {
    let id = egui::Id::new("tex_checker_texture");
    if let Some(handle) = ui.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    let (light, dark) = (color::TEXTURE_CHECKER_LIGHT, color::TEXTURE_CHECKER_DARK);
    let image = egui::ColorImage {
        size: [2, 2],
        pixels: vec![light, dark, dark, light],
    };
    let options = egui::TextureOptions {
        wrap_mode: egui::TextureWrapMode::Repeat,
        ..egui::TextureOptions::NEAREST
    };
    let handle = ui.ctx().load_texture("tex_checker", image, options);
    ui.data_mut(|data| data.insert_temp(id, handle.clone()));
    handle
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
