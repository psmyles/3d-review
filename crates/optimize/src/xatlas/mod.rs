//! The vendored xatlas (`third_party/xatlas`), behind the Unwrap operation.
//! [`ffi`] is the whole of the `unsafe`; everything here is safe.

#[cfg(has_xatlas)]
mod ffi;

#[cfg(has_xatlas)]
pub(crate) use ffi::{ChartOptions, Input, NO_CHART, Output, PackOptions, atlas};

/// Whether this build has xatlas compiled in.
pub(crate) const fn available() -> bool {
    cfg!(has_xatlas)
}
