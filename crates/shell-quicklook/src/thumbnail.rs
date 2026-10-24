//! The thumbnail: Finder hands the extension a file URL and a size, and draws
//! the straight-alpha image this returns over its own background.
//!
//! Unlike Explorer's stream-initialized handler, the extension is handed the
//! file's *path*, so textures resolve against its folder exactly as the preview's
//! do — as far as the extension's sandbox lets it read them (see the entitlements
//! beside `extension/`).

#![allow(
    unsafe_code,
    reason = "invariant 9: reading the path and filling the image the extension passes in"
)]

use std::ffi::{CStr, c_char};
use std::path::Path;

use review_preview::shared::with_host;
use review_preview::{PreviewScene, SceneSource, render_thumbnail};

use crate::guard::guarded;
use crate::image::ReviewQlImage;

/// The largest file read for a thumbnail. Past it Finder shows the ordinary
/// icon: a file that large is minutes of parsing for a picture a few dozen
/// pixels across.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// The largest thumbnail drawn. Finder asks for up to 1024 points at the
/// display's scale, which is a lot of supersampled GPU work for a tile shown at
/// a fraction of that; it scales a smaller image up itself.
const MAX_SIZE: u32 = 1024;

/// Draw `path`'s thumbnail into `out`. See the header.
///
/// # Safety
///
/// `path` is null or a NUL-terminated string, and `out` is null or points to a
/// writable `ReviewQlImage`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_thumbnail(
    path: *const c_char,
    side: u32,
    out: *mut ReviewQlImage,
) -> bool {
    // SAFETY: the caller's contract for both pointers.
    let (Some(out), false) = (unsafe { out.as_mut() }, path.is_null()) else {
        return false;
    };
    *out = ReviewQlImage::EMPTY;
    // SAFETY: non-null, and NUL-terminated by the caller's contract.
    let path = unsafe { CStr::from_ptr(path) };
    let Ok(path) = path.to_str() else {
        return false;
    };
    match guarded("thumbnail", None, || thumbnail(Path::new(path), side)) {
        Some(image) => {
            *out = image;
            true
        }
        None => false,
    }
}

fn thumbnail(path: &Path, side: u32) -> Option<ReviewQlImage> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_FILE_BYTES => {
            log::info!(
                "thumbnail skipped: {} is {} bytes, past the {MAX_FILE_BYTES} limit",
                path.display(),
                metadata.len()
            );
            return None;
        }
        Ok(_) => {}
        Err(error) => {
            log::info!("no thumbnail for {}: {error}", path.display());
            return None;
        }
    }
    let size = side.clamp(1, MAX_SIZE);
    let started = std::time::Instant::now();
    let scene = PreviewScene::load(
        SceneSource::Path(path),
        // Twice the tile, because it is drawn supersampled.
        Some((size * 4).clamp(512, 2048)),
    )
    .inspect_err(|error| log::info!("no thumbnail for {}: {error}", path.display()))
    .ok()?;
    let image = match with_host(|host| render_thumbnail(host, &scene, size)) {
        Ok(Ok(image)) => image,
        Ok(Err(error)) => {
            log::error!("thumbnail of {} did not draw: {error}", scene.name());
            return None;
        }
        Err(error) => {
            log::error!("thumbnail of {} did not draw: {error}", scene.name());
            return None;
        }
    };
    log::debug!(
        "thumbnail of {} at {size}px in {:.0} ms",
        scene.name(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    Some(ReviewQlImage::owned(image))
}
