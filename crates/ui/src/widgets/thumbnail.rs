//! Small previews of pooled textures, shared by every list that shows one — the
//! material Inspector's texture files, the Textures tab's rows and the texture
//! Inspector's preview.

use std::sync::Arc;

use review_render::DecodedImage;

use crate::state::TexturePoolEntry;

/// Lazily build + cache an egui texture thumbnail of a pooled image whose longest
/// edge is shown at `edge` points. Cached in egui temp data keyed by path and
/// size, with the source `Arc`'s identity stored so a disk reload (a fresh `Arc`
/// at the same path) rebuilds the thumbnail. Mirrors
/// [`crate::assets::load_icon_texture`]'s caching, downscaled on the CPU first so
/// a 4K source doesn't upload at full size.
pub(crate) fn texture_thumbnail(
    ui: &mut egui::Ui,
    entry: &TexturePoolEntry,
    edge: f32,
) -> Option<egui::load::SizedTexture> {
    // Twice the on-screen edge, so the thumbnail stays crisp on a HiDPI display.
    let max_edge = (edge * 2.0).round().max(1.0) as u32;
    let id = egui::Id::new(("texture_thumb", entry.path.as_path(), max_edge));
    let identity = Arc::as_ptr(&entry.image) as usize;
    if let Some((cached, handle)) =
        ui.data(|data| data.get_temp::<(usize, egui::TextureHandle)>(id))
        && cached == identity
    {
        return Some(egui::load::SizedTexture::from_handle(&handle));
    }

    let color_image = thumbnail_color_image(&entry.image, max_edge)?;
    let handle = ui.ctx().load_texture(
        format!("thumb{max_edge}:{}", entry.path.display()),
        color_image,
        egui::TextureOptions::LINEAR,
    );
    let sized = egui::load::SizedTexture::from_handle(&handle);
    ui.data_mut(|data| data.insert_temp(id, (identity, handle)));
    Some(sized)
}

/// Downscale a decoded RGBA8 image to an `egui::ColorImage` whose longest edge is
/// at most `max_edge` pixels, preserving aspect ratio. Returns `None` if the
/// buffer is too small to be that image.
fn thumbnail_color_image(image: &DecodedImage, max_edge: u32) -> Option<egui::ColorImage> {
    let width = image.width.max(1);
    let height = image.height.max(1);
    if image.rgba.len() < (width as usize * height as usize * 4) {
        return None;
    }
    let source = image::RgbaImage::from_raw(width, height, image.rgba.clone())?;
    let scale = (max_edge as f32 / width.max(height) as f32).min(1.0);
    let target_w = ((width as f32 * scale).round() as u32).max(1);
    let target_h = ((height as f32 * scale).round() as u32).max(1);
    let thumb = image::imageops::thumbnail(&source, target_w, target_h);
    let dimensions = [thumb.width() as usize, thumb.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        dimensions,
        thumb.as_raw(),
    ))
}
