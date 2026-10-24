//! Explorer's thumbnails and preview pane for FBX files: a COM in-process server
//! over [`review_preview`], so what Explorer shows is drawn by the viewer's own
//! renderer, with the textures the file names.
//!
//! Two classes, one DLL (`product.json`'s `shellExtension` names them both):
//!
//! * a **thumbnail provider** (`IThumbnailProvider` + `IInitializeWithStream`),
//!   which Explorer runs in its isolated COM Surrogate and hands the file as a
//!   stream of bytes — [`thumbnail`];
//! * a **preview handler** (`IPreviewHandler` and friends), which Explorer runs
//!   in a `prevhost.exe` of our own AppID and gives a child window in the
//!   preview pane — [`preview`]. Orbit, pan and zoom work there as in the viewer.
//!
//! Both draw through the process's one [`review_preview::Host`], kept in
//! [`host`] behind a lock and dropped again once the process has been idle a
//! while, so a surrogate that outlives the folder it drew holds no GPU memory.
//!
//! **Nothing here may panic out to the caller.** Every entry point Explorer
//! calls runs inside [`guard::guarded`], which turns a panic into an `HRESULT`;
//! the window procedure does the same. A panic unwinding into a COM vtable shim
//! would abort a process Explorer owns — which is also why this crate is built
//! with the `release-shell` profile (`panic = "unwind"`), not the workspace's
//! aborting `release`.
//!
//! All of the crate's `unsafe` is Win32 and COM plumbing, in the modules that
//! opt back in below; the scene, the camera and the pixels are safe code in
//! `review-preview` (invariant 9).

#![cfg(windows)]
#![deny(unsafe_code)]

mod com;
mod guard;
mod host;
mod preview;
mod thumbnail;

mod ids {
    include!(concat!(env!("OUT_DIR"), "/ids.rs"));
}

mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

/// The product name, from `product.json`: the log folder is named after it.
const PRODUCT: &str = env!("REVIEW_PRODUCT");

/// Set the process up for us, once: the log, and the message catalog.
fn initialize_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        review_log::install(env!("CARGO_CRATE_NAME"));
        if let Some(dir) = review_log::standard_dir(PRODUCT)
            && let Err(error) = review_log::open_file(&dir)
        {
            log::warn!("shell extension log file: {error}");
        }
        review_localization::init(None);
        log::info!(
            "{PRODUCT} shell extension {} loaded into process {}",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        );
    });
}
