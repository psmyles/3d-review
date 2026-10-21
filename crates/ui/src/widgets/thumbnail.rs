//! Small previews of pooled textures, shared by every list that shows one — the
//! material Inspector's texture files, the Textures tab's rows and the texture
//! Inspector's preview.
//!
//! The previews live in one bounded cache rather than an egui temp entry each.
//! egui never collects its temp data, so an entry per (path, size) kept every
//! preview ever drawn until exit: those of removed textures, of earlier models,
//! and one per frame while the Inspector was dragged narrower, since the preview
//! is sized from the panel's width. Each of those had also cloned the whole
//! source image to downscale it — 64 MB for a 4K map, on the UI thread.

use std::sync::{Arc, Weak};

use review_render::DecodedImage;

use crate::state::TexturePoolEntry;

/// How many previews the cache keeps before it starts dropping the least
/// recently drawn. Never one that is on screen, though: a list showing more
/// previews than this at once keeps them all, rather than rebuilding some of
/// them every frame.
const CAPACITY: usize = 64;

/// The pixel edges a preview is built at: every power of two from the first to
/// the second. Sizing to the power of two at or above what the widget asks for
/// keeps a preview between one and two texels per pixel — sharp, and inside what
/// bilinear filtering minifies cleanly — while a panel being dragged through a
/// hundred widths asks for only a few distinct sizes.
const EDGE_RANGE: (u32, u32) = (16, 1024);

/// Lazily build + cache an egui texture thumbnail of a pooled image whose longest
/// edge is shown at `edge` points. Mirrors [`crate::assets::load_icon_texture`]'s
/// lazy upload, downscaled on the CPU first so a 4K source doesn't upload at full
/// size. The texture handed back may be larger than `edge`: callers fit it to the
/// size they draw at.
pub(crate) fn texture_thumbnail(
    ui: &mut egui::Ui,
    entry: &TexturePoolEntry,
    edge: f32,
) -> Option<egui::load::SizedTexture> {
    let edge_px = bucket_edge(edge * ui.ctx().pixels_per_point());
    let pass = ui.ctx().cumulative_pass_nr();
    let id = cache_id();
    if let Some(handle) = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<ThumbnailCache>(id)
            .find(&entry.image, edge_px, pass)
    }) {
        return Some(egui::load::SizedTexture::from_handle(&handle));
    }

    // Built outside the `data_mut` above: uploading takes the context's lock
    // again, which egui would deadlock on.
    let color_image = thumbnail_color_image(&entry.image, edge_px)?;
    let handle = ui.ctx().load_texture(
        format!("thumb{edge_px}:{}", entry.path.display()),
        color_image,
        egui::TextureOptions::LINEAR,
    );
    let sized = egui::load::SizedTexture::from_handle(&handle);
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<ThumbnailCache>(id)
            .insert(Thumbnail {
                image: Arc::downgrade(&entry.image),
                edge: edge_px,
                handle,
                last_used: pass,
            });
    });
    Some(sized)
}

fn cache_id() -> egui::Id {
    egui::Id::new("texture_thumbnails")
}

/// The pixel edge a preview of `pixels` is built at; see [`EDGE_RANGE`].
fn bucket_edge(pixels: f32) -> u32 {
    let (smallest, largest) = EDGE_RANGE;
    let wanted = if pixels.is_finite() {
        pixels.ceil().max(1.0) as u32
    } else {
        smallest
    };
    wanted.clamp(smallest, largest).next_power_of_two()
}

/// Every preview built so far, and when each was last drawn.
#[derive(Clone, Default)]
struct ThumbnailCache {
    entries: Vec<Thumbnail>,
    /// The last pass that drew a preview, which is also the last one
    /// [`ThumbnailCache::sweep`] ran in. What it drew is what is on screen.
    swept_pass: Option<u64>,
}

#[derive(Clone)]
struct Thumbnail {
    /// The image this was built from, held weakly: the pool owns the pixels, and
    /// a preview must not keep a removed texture alive. A `Weak` still pins the
    /// allocation it points at, so no image decoded later can arrive at the same
    /// address and pass for this one — which an `Arc::as_ptr` kept as a number
    /// could not promise.
    image: Weak<DecodedImage>,
    /// The pixel edge it was built at.
    edge: u32,
    /// Dropping the handle frees the texture.
    handle: egui::TextureHandle,
    /// The pass it was last drawn in.
    last_used: u64,
}

impl ThumbnailCache {
    /// The preview of `image` at `edge`, marked as drawn in `pass`.
    fn find(
        &mut self,
        image: &Arc<DecodedImage>,
        edge: u32,
        pass: u64,
    ) -> Option<egui::TextureHandle> {
        self.sweep(pass);
        let thumbnail = self.entries.iter_mut().find(|thumbnail| {
            thumbnail.edge == edge && std::ptr::eq(thumbnail.image.as_ptr(), Arc::as_ptr(image))
        })?;
        thumbnail.last_used = pass;
        Some(thumbnail.handle.clone())
    }

    /// Add a freshly built preview. Trimming waits for the next pass's
    /// [`ThumbnailCache::sweep`]: mid-pass, an entry not yet drawn may be about to
    /// be, and evicting it would only rebuild it a moment later.
    fn insert(&mut self, thumbnail: Thumbnail) {
        self.entries.push(thumbnail);
    }

    /// Once a pass, the first time a preview is asked for: drop the previews of
    /// images nobody holds any more — a texture removed from the pool, reloaded
    /// from disk, or belonging to an earlier model — then the least recently drawn
    /// until the cache is back within [`CAPACITY`], sparing whatever the last
    /// drawing pass showed.
    fn sweep(&mut self, pass: u64) {
        if self.swept_pass == Some(pass) {
            return;
        }
        let on_screen = self.swept_pass.replace(pass);
        self.entries
            .retain(|thumbnail| thumbnail.image.strong_count() > 0);
        while self.entries.len() > CAPACITY {
            let Some(oldest) = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, thumbnail)| Some(thumbnail.last_used) != on_screen)
                .min_by_key(|(_, thumbnail)| thumbnail.last_used)
                .map(|(index, _)| index)
            else {
                break;
            };
            self.entries.swap_remove(oldest);
        }
    }
}

/// Downscale a decoded RGBA8 image to an `egui::ColorImage` whose longest edge is
/// at most `max_edge` pixels, preserving aspect ratio. Returns `None` if the
/// buffer is too small to be that image.
///
/// Reads the pooled pixels in place: the source is borrowed into an
/// `ImageBuffer` view rather than copied into an owned one.
fn thumbnail_color_image(image: &DecodedImage, max_edge: u32) -> Option<egui::ColorImage> {
    let width = image.width.max(1);
    let height = image.height.max(1);
    let len = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    let source = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(
        width,
        height,
        image.rgba.get(..len)?,
    )?;
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(name: &str) -> TexturePoolEntry {
        let (width, height) = (64, 32);
        TexturePoolEntry {
            path: PathBuf::from(name),
            image: Arc::new(DecodedImage {
                width,
                height,
                rgba: vec![128; (width * height * 4) as usize],
                source_channels: 4,
                source_bit_depth: 8,
            }),
            file_size: 0,
        }
    }

    /// One pass of a headless context, drawing `draw`.
    fn pass(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) {
        let output = ctx.run_ui(egui::RawInput::default(), draw);
        // No renderer here, so the texture uploads are deliberately dropped.
        output.drop_without_applying_deltas();
    }

    fn cached(ctx: &egui::Context) -> usize {
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<ThumbnailCache>(cache_id())
                .entries
                .len()
        })
    }

    /// Dragging the Inspector through every width it can take asks for a preview
    /// at each of them. Each used to be its own cache entry, kept until exit.
    #[test]
    fn resizing_a_preview_builds_only_a_few_sizes() {
        let ctx = egui::Context::default();
        let entry = entry("albedo.png");
        for edge in 1..=256 {
            pass(&ctx, |ui| {
                texture_thumbnail(ui, &entry, edge as f32).expect("a preview");
            });
        }
        let built = cached(&ctx);
        assert!(
            built <= 9,
            "256 widths built {built} previews; they should share a power-of-two size"
        );
    }

    /// A preview is dropped once its image is: the pool no longer holds it, so
    /// nothing will draw it again.
    #[test]
    fn a_removed_texture_frees_its_preview() {
        let ctx = egui::Context::default();
        let kept = entry("kept.png");
        let removed = entry("removed.png");
        pass(&ctx, |ui| {
            texture_thumbnail(ui, &kept, 20.0).expect("a preview");
            texture_thumbnail(ui, &removed, 20.0).expect("a preview");
        });
        assert_eq!(cached(&ctx), 2);

        drop(removed);
        pass(&ctx, |ui| {
            texture_thumbnail(ui, &kept, 20.0).expect("a preview");
        });
        assert_eq!(cached(&ctx), 1, "the removed texture's preview survived");
    }

    /// A reload is a new image at the same path: it must be a new preview, never
    /// the old one served because the two happen to compare equal.
    #[test]
    fn a_reloaded_texture_gets_a_new_preview() {
        let ctx = egui::Context::default();
        let first = entry("albedo.png");
        let mut ids = Vec::new();
        pass(&ctx, |ui| {
            ids.push(texture_thumbnail(ui, &first, 20.0).expect("a preview").id);
        });
        drop(first);
        let reloaded = entry("albedo.png");
        pass(&ctx, |ui| {
            ids.push(
                texture_thumbnail(ui, &reloaded, 20.0)
                    .expect("a preview")
                    .id,
            );
        });
        assert_ne!(ids[0], ids[1], "the reload was served the old preview");
    }

    /// The cache is bounded, but never at the cost of a preview drawn this frame:
    /// evicting one of those would rebuild it next frame, and every frame after.
    #[test]
    fn the_cache_is_bounded_without_evicting_what_is_on_screen() {
        let ctx = egui::Context::default();
        let entries: Vec<_> = (0..CAPACITY + 8)
            .map(|index| entry(&format!("{index}.png")))
            .collect();
        // Each drawn in a pass of its own, so only the one on screen is spared.
        // The trim runs as a pass begins, so the cache can hold one more than
        // its capacity: the preview that pass went on to build.
        for entry in &entries {
            pass(&ctx, |ui| {
                texture_thumbnail(ui, entry, 20.0).expect("a preview");
            });
        }
        assert_eq!(cached(&ctx), CAPACITY + 1);

        // All of them at once, twice: none may be evicted while it is on screen,
        // or the second pass would have to rebuild it.
        for _ in 0..2 {
            pass(&ctx, |ui| {
                for entry in &entries {
                    texture_thumbnail(ui, entry, 20.0).expect("a preview");
                }
            });
            assert_eq!(cached(&ctx), entries.len());
        }
        let ids = |ctx: &egui::Context| -> Vec<egui::TextureId> {
            let mut ids = Vec::new();
            pass(ctx, |ui| {
                for entry in &entries {
                    ids.push(texture_thumbnail(ui, entry, 20.0).expect("a preview").id);
                }
            });
            ids
        };
        assert_eq!(ids(&ctx), ids(&ctx), "a preview on screen was rebuilt");
    }

    #[test]
    fn a_preview_edge_is_a_power_of_two_at_or_above_the_ask() {
        assert_eq!(bucket_edge(40.0), 64);
        assert_eq!(bucket_edge(64.0), 64);
        assert_eq!(bucket_edge(65.0), 128);
        assert_eq!(bucket_edge(0.0), EDGE_RANGE.0);
        assert_eq!(bucket_edge(f32::NAN), EDGE_RANGE.0);
        assert_eq!(bucket_edge(1e9), EDGE_RANGE.1);
    }
}
