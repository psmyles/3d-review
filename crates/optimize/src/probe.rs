//! Test-only access to the vendored writer's patch probe (`src/ufbxw_probe.c`).
//!
//! The vendored ufbx_write carries this project's patches
//! (`third_party/ufbx-write/review.patch`): user-property flags, the `Null`
//! attribute, the topology layers and the LOD group / layered texture. The
//! probe writes one scene using each, and `tests/ufbxw_patches.rs` reads the
//! file back through the vendored ufbx reader. It exists only under
//! `cfg(has_ufbxw_probe)` — non-release profiles — and is `doc(hidden)` because
//! nothing but that test should call it.

use std::ffi::{CString, c_char, c_int};
use std::path::Path;

use crate::export_ffi;

/// Write the probe scene to `path`, binary or ASCII.
pub fn write_patch_probe(path: &Path, ascii: bool) -> Result<(), String> {
    let path_string = path
        .to_str()
        .ok_or_else(|| "the output path is not valid UTF-8".to_owned())?;
    let c_path =
        CString::new(path_string).map_err(|_| "the output path contains NUL".to_owned())?;
    let mut error = vec![0u8; export_ffi::ERROR_LENGTH];

    // SAFETY: `c_path` is NUL-terminated and outlives the call; `error` has
    // exactly `ERROR_LENGTH` bytes, which is the length passed. The probe
    // borrows nothing past the call and frees its own scene on every path.
    let status = unsafe {
        export_ffi::review_ufbxw_patch_probe(
            c_path.as_ptr(),
            c_int::from(ascii),
            error.as_mut_ptr().cast::<c_char>(),
            error.len(),
        )
    };
    if status == 0 {
        return Ok(());
    }
    let end = error.iter().position(|&b| b == 0).unwrap_or(error.len());
    Err(String::from_utf8_lossy(&error[..end]).into_owned())
}
