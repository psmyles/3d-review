//! Where the manual's images live on this install.
//!
//! Only images are shipped on disk. Every page's *text* is compiled into the
//! binary (invariant 12), so Help works with no docs folder at all — an image
//! that is not there renders as its alt text and nothing else changes. That is
//! also why this returns an `Option` rather than failing: a missing folder is a
//! degraded manual, not a broken viewer.

use std::path::PathBuf;

/// The manual's asset directory, or `None` when this install has none.
///
/// Three places are tried, in the order an install can be shaped:
///
/// 1. `docs/` beside the executable — what the Windows installer lays down.
/// 2. `../Resources/docs` — the macOS bundle, where `Contents/MacOS` holds the
///    binary and `Contents/Resources` its data.
/// 3. the repository's own `docs/book/src` — so a `cargo run` from a checkout
///    shows the images without an install step. The path is compiled in, which
///    is harmless in a shipped binary: it is a string that names a directory
///    that will not exist there, and the check above it will already have hit.
pub(crate) fn docs_dir() -> Option<PathBuf> {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from));

    let candidates = [
        beside_exe.as_ref().map(|dir| dir.join("docs")),
        beside_exe.as_ref().map(|dir| dir.join("../Resources/docs")),
        Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/book/src"
        ))),
    ];

    candidates.into_iter().flatten().find(|path| path.is_dir())
}
