//! FFI to the psd_sdk C++ library (Molecular Matters) via a thin `extern "C"`
//! C-ABI wrapper, exposing a small safe Rust API that decodes a PSD's
//! merged/composited image to 8-bit RGBA.
//!
//! This is the FBX importer's sibling: the **only** place besides `crates/import`
//! and the sanctioned Direct3D 11 sites where `unsafe`/FFI lives (invariant 9). It
//! exists so the texture pipeline can read layered PSD source art without shelling
//! out to a bundled ImageMagick — the merged composite ("Maximize Compatibility")
//! is what the viewer displays.
//!
//! The psd_sdk C++ is **vendored as source** (`vendor/Psd/`) and compiled by `cc`
//! alongside the C-ABI `src/wrapper.cpp` — the same model as ufbx
//! (`crates/import`) and meshoptimizer (`crates/optimize`), so a clean checkout
//! builds on any OS with a C++ compiler. `src/bindings.rs` is committed Rust kept
//! in lockstep with `src/wrapper.h` by hand, so no bindgen or libclang runs at
//! build time — see `build.rs` / `vendor/NOTICE.txt`.
//!
//! Safety: [`decode_psd`] validates the header dimensions with checked arithmetic
//! before sizing the output buffer, so a malformed header can never wrap to an
//! undersized allocation the C++ merged-image read would then overrun. The C++
//! wrapper additionally catches its own exceptions at the FFI boundary. Callers
//! (the texture decode worker in `crates/app`) run this off the UI thread.

use thiserror::Error;

mod ffi {
    // The committed bindgen output declares the full C-ABI surface, incl. the ICC
    // helpers this viewer doesn't use (it has no ICC pipeline). Keep them bound but
    // silence the unused-function + generated-naming warnings here, scoped to the
    // generated code, rather than hand-editing it (or de-linting the whole crate).
    #![allow(dead_code)]
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    include!("bindings.rs");
}

/// A decoded PSD merged image, normalized to 8-bit RGBA.
#[derive(Debug, Clone)]
pub struct PsdImage {
    pub width: u32,
    pub height: u32,
    /// Source channel count (incl. extra alpha channels) — for the stats panel.
    pub channels: u16,
    /// Source bits per channel (8/16/32) — for the stats panel.
    pub bits_per_channel: u16,
    /// Interleaved RGBA, 8-bit, row-major (`width * height * 4` bytes).
    pub rgba8: Vec<u8>,
}

/// Why a PSD decode failed.
#[derive(Debug, Error)]
pub enum PsdError {
    /// psd_sdk could not parse the file / out of memory.
    #[error("psd_sdk failed to open the document")]
    OpenFailed,
    /// Header info could not be read (incl. zero/degenerate dimensions).
    #[error("could not read PSD header info")]
    InfoFailed,
    /// The header's dimensions exceed the decode ceiling — a malformed (or
    /// absurd) file whose output buffer would be a multi-gigabyte allocation.
    #[error("PSD merged image is too large to decode ({width}x{height})")]
    TooLarge { width: u32, height: u32 },
    /// No merged image — the PSD was saved without "Maximize Compatibility".
    #[error("PSD has no composited image (re-save with Maximize Compatibility)")]
    NoMergedImage,
    /// Unexpected non-zero return from the merged-image read (code).
    #[error("PSD merged-image read failed (code {0})")]
    ReadFailed(i32),
}

/// RAII guard that frees the opaque psd document on drop (incl. early returns).
struct DocHandle(*mut ffi::fire_psd);

impl Drop for DocHandle {
    fn drop(&mut self) {
        // SAFETY: pointer came from fire_psd_open and is freed exactly once.
        unsafe { ffi::fire_psd_free(self.0) };
    }
}

/// Hard ceiling on either decoded dimension. PSD's own format maximum is
/// 30000 px (PSB: 300000), and source-art textures are far smaller; a header
/// past this is malformed or absurd, and is rejected instead of attempted.
const MAX_DIMENSION: u32 = 30000;

/// Hard ceiling on the decoded RGBA payload (1 GiB — a 16384² RGBA image).
/// The dimension check alone still admits a ~3.6 GB allocation whose failure
/// would abort the whole process; cap the total instead of gambling on it.
const MAX_OUTPUT_BYTES: usize = 1 << 30;

/// Validate a merged image's header dimensions and size its RGBA8 output
/// buffer. FFI = validation boundary: rejects zero/degenerate/absurd dimensions
/// and sizes with checked arithmetic, so a malformed header can neither wrap to
/// an undersized allocation the C++ merged-image read then overruns, nor demand
/// a multi-gigabyte allocation whose failure aborts the process.
fn checked_output_len(width: u32, height: u32) -> Result<usize, PsdError> {
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(PsdError::TooLarge { width, height });
    }
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|&n| n != 0)
        .ok_or(PsdError::InfoFailed)?;
    if len > MAX_OUTPUT_BYTES {
        return Err(PsdError::TooLarge { width, height });
    }
    Ok(len)
}

/// Decode a PSD's merged image from in-memory bytes into 8-bit RGBA.
pub fn decode_psd(bytes: &[u8]) -> Result<PsdImage, PsdError> {
    // SAFETY: `bytes`/len describe a valid read-only slice for the duration of
    // the call; the C++ side copies what it needs internally and holds no
    // reference past the return.
    let handle = unsafe { ffi::fire_psd_open(bytes.as_ptr(), bytes.len()) };
    if handle.is_null() {
        return Err(PsdError::OpenFailed);
    }
    // Frees the document exactly once, on every return path below.
    let guard = DocHandle(handle);

    let mut info = ffi::fire_psd_info {
        width: 0,
        height: 0,
        channels: 0,
        bits_per_channel: 0,
    };
    // SAFETY: `handle` is the live document just opened (guard not dropped);
    // `info` is a valid out-pointer to a correctly-sized struct the call fully
    // writes on success.
    if unsafe { ffi::fire_psd_info_get(handle, &mut info) } != 0 {
        return Err(PsdError::InfoFailed);
    }

    let (w, h) = (info.width, info.height);
    let len = checked_output_len(w, h)?;
    let mut rgba = vec![0u8; len];
    // SAFETY: `handle` is still live (guard not dropped); `rgba` is a writable
    // buffer of exactly `len` bytes, and `len` was sized from the same
    // `fire_psd_info_get` dimensions the wrapper decodes, so a full `w*h*4`
    // merged-image write cannot overrun it — the wrapper honors the passed
    // `out_len` as its write bound regardless.
    let rc = unsafe { ffi::fire_psd_read_merged_rgba8(handle, rgba.as_mut_ptr(), rgba.len()) };
    match rc {
        0 => {}
        2 => return Err(PsdError::NoMergedImage),
        other => return Err(PsdError::ReadFailed(other)),
    }

    drop(guard); // explicit: free the document now that pixels are copied out
    Ok(PsdImage {
        width: w,
        height: h,
        channels: info.channels,
        bits_per_channel: info.bits_per_channel,
        rgba8: rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_len_accepts_normal_dimensions() {
        assert_eq!(checked_output_len(4, 8).unwrap(), 4 * 8 * 4);
        // The largest square the byte cap admits (16384² × 4 = 1 GiB) passes.
        assert!(checked_output_len(16_384, 16_384).is_ok());
    }

    #[test]
    fn output_len_rejects_zero_and_absurd_dimensions() {
        assert!(matches!(
            checked_output_len(0, 128),
            Err(PsdError::InfoFailed)
        ));
        // Past the per-dimension ceiling (a malformed header).
        assert!(matches!(
            checked_output_len(MAX_DIMENSION + 1, 16),
            Err(PsdError::TooLarge { .. })
        ));
        // Within the dimension ceiling but past the total-byte cap: the
        // allocation that would abort the process on failure is refused.
        assert!(matches!(
            checked_output_len(30_000, 30_000),
            Err(PsdError::TooLarge { .. })
        ));
    }

    #[test]
    fn garbage_bytes_fail_to_open_cleanly() {
        // The full FFI path: not a PSD → OpenFailed, never a crash or leak
        // (the DocHandle guard frees on every path).
        assert!(matches!(
            decode_psd(b"not a psd"),
            Err(PsdError::OpenFailed)
        ));
        assert!(matches!(decode_psd(&[]), Err(PsdError::OpenFailed)));
    }
}
