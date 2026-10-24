//! Turning a panic into a plain "no" at every boundary Swift calls across.

use std::panic::{AssertUnwindSafe, catch_unwind};

/// Run `body`, returning its result — or `fallback`, logged, if it panicked.
///
/// `AssertUnwindSafe` is sound here because nothing a body touches is left for
/// the next call to trust after a panic: the host is dropped and rebuilt
/// (`review_preview::shared::with_host` has a guard of its own), and a preview
/// whose call panicked keeps only state its next call overwrites or ignores.
pub(crate) fn guarded<T>(what: &str, fallback: T, body: impl FnOnce() -> T) -> T {
    crate::initialize_once();
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(_) => {
            log::error!("{what} panicked");
            fallback
        }
    }
}
