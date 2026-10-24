//! Finder's thumbnails and Quick Look preview for FBX files: a C-ABI dylib over
//! [`review_preview`], so what Finder shows is drawn by the viewer's own
//! renderer, with the textures the file names. The twin of `shell-windows`,
//! without COM.
//!
//! Two small Swift app extensions call it (`extension/`, built and bundled by
//! `scripts/build-quicklook.sh`), and both link the one copy of it in the app's
//! `Contents/Frameworks`:
//!
//! * a **thumbnail provider** (`QLThumbnailProvider`), handed a file URL and a
//!   size — [`thumbnail`];
//! * a **view-based preview** (`QLPreviewingController`), which Finder shows in
//!   the Quick Look panel and the column view's preview, and where orbit, pan and
//!   zoom work as in the viewer — [`preview`].
//!
//! Both draw through the process's one host ([`review_preview::shared`]), which
//! is dropped again once the extension has been idle a while.
//!
//! The ABI is `include/review_quicklook.h`, hand-kept against the `extern "C"`
//! functions in the modules below. **Nothing here may panic out to the
//! caller**: every entry point runs inside [`guard::guarded`], since unwinding
//! into Swift is undefined and an abort would take the extension down with it —
//! which is also why this crate is built with the `release-shell` profile
//! (`panic = "unwind"`).
//!
//! All of the crate's `unsafe` is reading and writing the pointers the
//! extensions pass across that ABI, in the modules that opt back in below; the
//! scene, the camera and the pixels are safe code in `review-preview`
//! (invariant 9).

#![cfg(target_os = "macos")]
#![deny(unsafe_code)]

mod guard;
mod image;
mod preview;
mod thumbnail;

mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

/// The product name, from `product.json`: the log folder is named after it.
const PRODUCT: &str = env!("REVIEW_PRODUCT");

/// Set the process up for us, once: the log, and the message catalog.
///
/// The extensions are sandboxed, so the log's `~/Library/Logs` is the
/// extension's own container (`~/Library/Containers/<extension id>/Data/
/// Library/Logs/3D Review`), not the viewer's folder.
fn initialize_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        review_log::install(env!("CARGO_CRATE_NAME"));
        if let Some(dir) = review_log::standard_dir(PRODUCT)
            && let Err(error) = review_log::open_file(&dir)
        {
            log::warn!("Quick Look extension log file: {error}");
        }
        review_localization::init(None);
        log::info!(
            "{PRODUCT} Quick Look extension {} loaded into process {}",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        );
    });
}
