//! FFI to the psd_sdk C++ library (Molecular Matters) via a thin `extern "C"`
//! C-ABI wrapper, exposing a small safe Rust API that decodes a PSD's
//! merged/composited image to 8-bit RGBA.
//!
//! This is the FBX importer's sibling: one of the sanctioned FFI sites (invariant 9),
//! beside `crates/import`, `crates/optimize` and the GPU backend leaf. It exists so the
//! texture pipeline can read layered PSD source art without shelling out to a bundled
//! ImageMagick — the merged composite ("Maximize Compatibility") is what the viewer
//! displays.
//!
//! The psd_sdk C++ is **vendored as source** (`vendor/Psd/`, patched — see
//! `vendor/NOTICE.txt`) and compiled by `cc` alongside the C-ABI `src/wrapper.cpp` — the
//! same model as ufbx (`crates/import`) and meshoptimizer (`crates/optimize`), so a
//! clean checkout builds on any OS with a C++ compiler. `src/bindings.rs` is committed
//! Rust kept in lockstep with `src/wrapper.h` by hand, so no bindgen or libclang runs at
//! build time — see `build.rs` / `vendor/NOTICE.txt`.
//!
//! Safety: the decode is staged so the header is read and **validated before anything
//! is sized from it** — `fire_psd_open` parses the header alone, [`decode_psd`] refuses
//! a document it cannot draw faithfully or whose buffers would be absurd
//! (`check_supported`, `checked_output_len`), and only then asks the wrapper to
//! allocate and decode the planes. The wrapper repeats the same checks on its side, so
//! the C ABI is safe on its own, and it catches its own exceptions at the boundary.
//! Callers (the texture decode worker in `crates/app`) run this off the UI thread.

use std::marker::PhantomData;

use thiserror::Error;

mod ffi {
    // The committed bindgen-shaped declarations; only the generated naming style is
    // silenced here, scoped to that code, rather than hand-renaming it.
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
    /// Planes the document stores, extra channels included (spot colours, saved
    /// selections). Not what the composite is drawn from: see
    /// [`Self::drawn_channels`].
    pub channels: u16,
    /// Whether the document is Grayscale rather than RGB — the only two modes
    /// the decode accepts.
    pub grayscale: bool,
    /// Source bits per channel (8/16/32) — for the stats panel.
    pub bits_per_channel: u16,
    /// Interleaved RGBA, 8-bit, row-major (`width * height * 4` bytes).
    pub rgba8: Vec<u8>,
}

impl PsdImage {
    /// The channels the composite is drawn from: 1 grey, 2 grey + alpha, 3 RGB,
    /// 4 RGBA — what the stats panel and the alpha tests downstream mean by a
    /// channel count.
    ///
    /// The merged read takes colour from the first plane (Grayscale) or three
    /// (RGB), and transparency from the plane after them when the document has
    /// one; every later plane is never read. Counting planes instead called a
    /// grey + alpha document with one saved selection RGB, losing its alpha, and an
    /// RGBA one with a spot channel five, which nothing can name.
    pub fn drawn_channels(&self) -> u8 {
        let colour = if self.grayscale { 1 } else { 3 };
        colour + u8::from(self.channels > u16::from(colour))
    }
}

/// Why a PSD decode failed.
#[derive(Debug, Error)]
pub enum PsdError {
    /// psd_sdk could not parse the header / out of memory.
    #[error("psd_sdk failed to open the document")]
    OpenFailed,
    /// Header info could not be read (incl. zero/degenerate dimensions).
    #[error("could not read PSD header info")]
    InfoFailed,
    /// The header's dimensions exceed the decode ceiling — a malformed (or
    /// absurd) file whose output buffer would be a multi-gigabyte allocation.
    #[error("PSD merged image is too large to decode ({width}x{height})")]
    TooLarge { width: u32, height: u32 },
    /// A document the viewer cannot draw faithfully: a colour mode other than
    /// Grayscale/RGB, a bit depth other than 8/16/32, or an impossible channel count.
    #[error("PSD {0} is not supported")]
    Unsupported(String),
    /// No merged image — the PSD was saved without "Maximize Compatibility".
    #[error("PSD has no composited image (re-save with Maximize Compatibility)")]
    NoMergedImage,
    /// The file ends before the data its header describes.
    #[error("PSD file is truncated")]
    Truncated,
    /// The merged image's pixel data is malformed (e.g. a corrupt RLE stream).
    #[error("PSD merged image data is corrupt")]
    DecodeFailed,
    /// The decoded image's buffer could not be allocated.
    #[error("not enough memory to decode the PSD merged image ({bytes} bytes)")]
    OutOfMemory { bytes: usize },
    /// Unexpected non-zero return from the wrapper (its status code).
    #[error("PSD merged-image read failed (code {0})")]
    ReadFailed(i32),
}

/// RAII guard that frees the opaque psd document on drop (incl. early returns).
///
/// The document reads the caller's bytes in place rather than copying the file
/// (`fire_psd_open`), so it borrows them: the lifetime is what makes the compiler
/// hold the slice alive, unmoved and unmodified, for as long as the handle exists.
struct DocHandle<'bytes> {
    handle: *mut ffi::fire_psd,
    _bytes: PhantomData<&'bytes [u8]>,
}

impl<'bytes> DocHandle<'bytes> {
    /// Open `bytes` as a document, or `None` when psd_sdk cannot parse its header.
    fn open(bytes: &'bytes [u8]) -> Option<Self> {
        // SAFETY: `bytes`/len describe a valid read-only slice. The document reads
        // it in place until `fire_psd_free`, which only the returned handle's drop
        // calls, and the handle borrows `bytes` for `'bytes` — so the slice
        // outlives every read.
        let handle = unsafe { ffi::fire_psd_open(bytes.as_ptr(), bytes.len()) };
        (!handle.is_null()).then_some(Self {
            handle,
            _bytes: PhantomData,
        })
    }
}

impl Drop for DocHandle<'_> {
    fn drop(&mut self) {
        // SAFETY: pointer came from fire_psd_open and is freed exactly once.
        unsafe { ffi::fire_psd_free(self.handle) };
    }
}

/// Hard ceiling on either decoded dimension. PSD's own format maximum is
/// 30000 px (PSB: 300000), and source-art textures are far smaller; a header
/// past this is malformed or absurd, and is rejected instead of attempted.
/// Mirrored by `MAX_PSD_DIM` in `wrapper.cpp`.
const MAX_DIMENSION: u32 = 30000;

/// Hard ceiling on the decoded RGBA payload (1 GiB — a 16384² RGBA image).
/// The dimension check alone still admits a ~3.6 GB buffer for a header that is
/// more likely malformed than meant; cap the total rather than attempt it. The
/// buffer under the cap is reserved fallibly ([`decode_psd`]), so a machine that
/// cannot spare it gets an error, not an aborted process. Mirrored by
/// `MAX_PSD_OUTPUT_BYTES` in `wrapper.cpp`.
const MAX_OUTPUT_BYTES: usize = 1 << 30;

/// Hard ceiling on the planar data psd_sdk holds while decoding (every channel at
/// the source depth) — 2 GiB, a 16384² RGBA document at 16 bits. Mirrored by
/// `MAX_PSD_PLANAR_BYTES` in `wrapper.cpp`.
const MAX_PLANAR_BYTES: u64 = 2 << 30;

/// Photoshop's own channel limit. Mirrored by `MAX_PSD_CHANNELS` in `wrapper.cpp`.
const MAX_CHANNELS: u16 = 56;

/// psd_sdk's `colorMode::Enum` values for the two modes the wrapper draws.
const COLOR_MODE_GRAYSCALE: u16 = 1;
const COLOR_MODE_RGB: u16 = 3;

/// Refuse a document the wrapper cannot draw faithfully, before anything is sized
/// from its header: the bit depths psd_sdk decodes (a 1-bit Bitmap document would get
/// zero-byte planes), the two colour modes the merged read understands (CMYK or Lab
/// drawn as RGB is a wrong picture, not an image), and a channel count Photoshop could
/// have written. The planar total is capped so the decode cannot ask for gigabytes.
fn check_supported(info: &ffi::fire_psd_info) -> Result<(), PsdError> {
    let unsupported = |what: &str| Err(PsdError::Unsupported(what.to_owned()));
    let mode = match info.color_mode {
        COLOR_MODE_GRAYSCALE | COLOR_MODE_RGB => info.color_mode,
        0 => return unsupported("Bitmap color mode"),
        2 => return unsupported("Indexed color mode"),
        4 => return unsupported("CMYK color mode"),
        7 => return unsupported("Multichannel color mode"),
        8 => return unsupported("Duotone color mode"),
        9 => return unsupported("Lab color mode"),
        other => return unsupported(&format!("color mode {other}")),
    };
    if !matches!(info.bits_per_channel, 8 | 16 | 32) {
        return unsupported(&format!("{}-bit depth", info.bits_per_channel));
    }
    let min_channels = if mode == COLOR_MODE_RGB { 3 } else { 1 };
    if info.channels < min_channels || info.channels > MAX_CHANNELS {
        return unsupported(&format!("channel count of {}", info.channels));
    }
    let planar = u64::from(info.width)
        * u64::from(info.height)
        * u64::from(info.channels)
        * u64::from(info.bits_per_channel / 8);
    if planar > MAX_PLANAR_BYTES {
        return Err(PsdError::TooLarge {
            width: info.width,
            height: info.height,
        });
    }
    Ok(())
}

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

/// The error a non-`FIRE_PSD_OK` wrapper status stands for.
fn status_error(status: i32) -> PsdError {
    match u32::try_from(status) {
        Ok(ffi::FIRE_PSD_NO_MERGED_IMAGE) => PsdError::NoMergedImage,
        Ok(ffi::FIRE_PSD_UNSUPPORTED) => {
            PsdError::Unsupported("document (refused by the decoder)".to_owned())
        }
        Ok(ffi::FIRE_PSD_DECODE_FAILED) => PsdError::DecodeFailed,
        Ok(ffi::FIRE_PSD_TRUNCATED) => PsdError::Truncated,
        // This crate's own mistakes rather than the file's; report the code verbatim.
        Ok(
            ffi::FIRE_PSD_BAD_ARGUMENT | ffi::FIRE_PSD_NOT_DECODED | ffi::FIRE_PSD_BUFFER_TOO_SMALL,
        ) => PsdError::ReadFailed(status),
        // A code this crate does not know: the wrapper and the bindings disagree.
        Ok(_) | Err(_) => PsdError::ReadFailed(status),
    }
}

/// Decode a PSD's merged image from in-memory bytes into 8-bit RGBA.
pub fn decode_psd(bytes: &[u8]) -> Result<PsdImage, PsdError> {
    // Frees the document exactly once, on every return path below.
    let guard = DocHandle::open(bytes).ok_or(PsdError::OpenFailed)?;
    let handle = guard.handle;

    let mut info = ffi::fire_psd_info {
        width: 0,
        height: 0,
        channels: 0,
        bits_per_channel: 0,
        color_mode: 0,
        reserved: 0,
    };
    // SAFETY: `handle` is the live document just opened (guard not dropped);
    // `info` is a valid out-pointer to a correctly-sized struct the call fully
    // writes on success.
    if unsafe { ffi::fire_psd_info_get(handle, &mut info) } != 0 {
        return Err(PsdError::InfoFailed);
    }

    let (w, h) = (info.width, info.height);
    let len = checked_output_len(w, h)?;
    check_supported(&info)?;

    // SAFETY: `handle` is still live (guard not dropped). The wrapper re-checks the
    // header against the same limits before it allocates anything.
    let status = unsafe { ffi::fire_psd_decode_merged(handle) };
    if status != ffi::FIRE_PSD_OK as i32 {
        return Err(status_error(status));
    }

    // Reserved fallibly: up to a gigabyte, and `vec![0; len]` aborts the process
    // when the allocator refuses, which is the gamble `MAX_OUTPUT_BYTES` exists to
    // avoid.
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(len)
        .map_err(|_| PsdError::OutOfMemory { bytes: len })?;
    rgba.resize(len, 0u8);
    // SAFETY: `handle` is still live and decoded; `rgba` is a writable buffer of
    // exactly `len` bytes, and `len` was sized from the same `fire_psd_info_get`
    // dimensions the wrapper decodes, so a full `w*h*4` merged-image write cannot
    // overrun it — the wrapper honors the passed `out_len` as its write bound
    // regardless.
    let status = unsafe { ffi::fire_psd_read_merged_rgba8(handle, rgba.as_mut_ptr(), rgba.len()) };
    if status != ffi::FIRE_PSD_OK as i32 {
        return Err(status_error(status));
    }

    drop(guard); // explicit: free the document now that pixels are copied out
    Ok(PsdImage {
        width: w,
        height: h,
        channels: info.channels,
        grayscale: info.color_mode == COLOR_MODE_GRAYSCALE,
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

    /// Only the planes the merged read draws from count: the colour, then one
    /// for transparency when there is a plane after the colour. A saved selection
    /// or spot channel beyond that is stored but never read.
    #[test]
    fn drawn_channels_count_colour_and_transparency_only() {
        let image = |grayscale, channels| PsdImage {
            width: 1,
            height: 1,
            channels,
            grayscale,
            bits_per_channel: 8,
            rgba8: vec![0; 4],
        };
        assert_eq!(image(true, 1).drawn_channels(), 1);
        assert_eq!(image(true, 2).drawn_channels(), 2);
        // Grey + alpha + one saved selection: still grey + alpha, not RGB.
        assert_eq!(image(true, 3).drawn_channels(), 2);
        assert_eq!(image(false, 3).drawn_channels(), 3);
        assert_eq!(image(false, 4).drawn_channels(), 4);
        // RGBA + a spot channel.
        assert_eq!(image(false, 5).drawn_channels(), 4);
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
