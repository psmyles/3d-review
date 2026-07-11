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
//! Unlike a normal `-sys` crate this one links a **prebuilt** static lib
//! (`vendor/fire_psd.lib`) and includes committed bindgen output
//! (`src/bindings.rs`) — see `build.rs` / `vendor/NOTICE.txt`. No C++ toolchain,
//! libclang, or bindgen runs at build time.
//!
//! Safety: [`decode_psd`] validates the header dimensions with checked arithmetic
//! before sizing the output buffer, so a malformed header can never wrap to an
//! undersized allocation the C++ merged-image read would then overrun. The C++
//! wrapper additionally catches its own exceptions at the FFI boundary. Callers
//! (the texture decode worker in `crates/app`) run this off the UI thread.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

mod ffi {
    // The committed bindgen output declares the full C-ABI surface, incl. the ICC
    // helpers this viewer doesn't use (it has no ICC pipeline). Keep them bound but
    // silence the unused-function warnings rather than hand-editing generated code.
    #![allow(dead_code)]
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
#[derive(Debug)]
pub enum PsdError {
    /// psd_sdk could not parse the file / out of memory.
    OpenFailed,
    /// Header info could not be read (incl. zero/degenerate dimensions).
    InfoFailed,
    /// No merged image — the PSD was saved without "Maximize Compatibility".
    NoMergedImage,
    /// Unexpected non-zero return from the merged-image read (code).
    ReadFailed(i32),
}

impl std::fmt::Display for PsdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PsdError::OpenFailed => write!(f, "psd_sdk failed to open the document"),
            PsdError::InfoFailed => write!(f, "could not read PSD header info"),
            PsdError::NoMergedImage => write!(
                f,
                "PSD has no composited image (re-save with Maximize Compatibility)"
            ),
            PsdError::ReadFailed(c) => write!(f, "PSD merged-image read failed (code {c})"),
        }
    }
}

impl std::error::Error for PsdError {}

/// RAII guard that frees the opaque psd document on drop (incl. early returns).
struct DocHandle(*mut ffi::fire_psd);

impl Drop for DocHandle {
    fn drop(&mut self) {
        // SAFETY: pointer came from fire_psd_open and is freed exactly once.
        unsafe { ffi::fire_psd_free(self.0) };
    }
}

/// Decode a PSD's merged image from in-memory bytes into 8-bit RGBA.
pub fn decode_psd(bytes: &[u8]) -> Result<PsdImage, PsdError> {
    // SAFETY: bytes/len describe a valid read-only slice for the duration of the
    // call; the C++ side copies them internally. The handle is freed by DocHandle
    // on every return path.
    unsafe {
        let handle = ffi::fire_psd_open(bytes.as_ptr(), bytes.len());
        if handle.is_null() {
            return Err(PsdError::OpenFailed);
        }
        let guard = DocHandle(handle);

        let mut info = ffi::fire_psd_info {
            width: 0,
            height: 0,
            channels: 0,
            bits_per_channel: 0,
        };
        if ffi::fire_psd_info_get(handle, &mut info) != 0 {
            return Err(PsdError::InfoFailed);
        }

        let (w, h) = (info.width, info.height);
        // FFI = validation boundary: reject zero/degenerate dimensions and size the
        // buffer with checked arithmetic, so a malformed header can never wrap to an
        // undersized allocation that the C++ merged-image read then overruns.
        let len = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|&n| n != 0)
            .ok_or(PsdError::InfoFailed)?;
        let mut rgba = vec![0u8; len];
        let rc = ffi::fire_psd_read_merged_rgba8(handle, rgba.as_mut_ptr(), rgba.len());
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
}
