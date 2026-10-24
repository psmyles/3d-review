//! Turning a panic into an `HRESULT` at every boundary Explorer calls across.

use std::panic::{AssertUnwindSafe, catch_unwind};

use windows::Win32::Foundation::E_UNEXPECTED;
use windows::core::{Error, Result};

/// Run `body`, returning its result — or `E_UNEXPECTED`, logged, if it panicked.
///
/// `AssertUnwindSafe` is sound here because nothing a body touches is left for
/// the next call to trust after a panic: the host is dropped and rebuilt
/// ([`crate::host::with_host`] has a guard of its own), and a COM object whose
/// call panicked keeps only plain values it overwrites on its next call.
pub(crate) fn guarded<T>(what: &str, body: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => {
            if let Err(error) = &result {
                log::debug!("{what}: {error}");
            }
            result
        }
        Err(_) => {
            log::error!("{what} panicked; answered E_UNEXPECTED");
            Err(Error::from_hresult(E_UNEXPECTED))
        }
    }
}
